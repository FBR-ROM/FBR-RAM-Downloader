//! What the program needs from the host: root, a lock, cancellation, and the
//! cards' BARs.

use std::fs::{File, OpenOptions};
use std::io;
use std::os::unix::fs::OpenOptionsExt;
use std::os::unix::io::AsRawFd;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};

use crate::bar0::{Access, Bar0, Regs};
use crate::card::{discover, Bdf, Card, SYSFS_PCI};
use crate::mmap::MmapBar0;
use crate::outcome::{Kind, Outcome};
use crate::run::Unit;

/// A card behind dynamic dispatch, so real and simulated cards share one path.
pub type AnyUnit<'a> = Unit<Box<dyn Bar0 + 'a>>;

/// In /run, which only root can write to, so no other user can pre-create
/// the lock or hold it to block a run.
const LOCK_PATH: &str = "/run/v100-unlock.lock";

static CANCELLED: AtomicBool = AtomicBool::new(false);

pub fn cancelled() -> bool {
    CANCELLED.load(Ordering::SeqCst)
}

extern "C" fn on_signal(_: libc::c_int) {
    CANCELLED.store(true, Ordering::SeqCst);
}

/// Cancel on SIGINT, SIGTERM or SIGHUP (a dropped ssh session) and, when a
/// front end drives the run, when it closes our stdin.
pub fn watch_for_cancel(watch_stdin: bool) {
    let handler = on_signal as extern "C" fn(libc::c_int) as libc::sighandler_t;
    for signal in [libc::SIGINT, libc::SIGTERM, libc::SIGHUP] {
        // SAFETY: the handler only stores to an atomic, which is async-signal-safe.
        unsafe { libc::signal(signal, handler) };
    }
    if watch_stdin {
        std::thread::spawn(|| {
            let _ = io::copy(&mut io::stdin().lock(), &mut io::sink());
            CANCELLED.store(true, Ordering::SeqCst);
        });
    }
}

pub fn require_root() -> Result<(), Outcome> {
    // SAFETY: geteuid has no preconditions and cannot fail.
    if unsafe { libc::geteuid() } == 0 {
        Ok(())
    } else {
        Err(refused(
            "needs root to reach the GPU registers: run it with sudo",
        ))
    }
}

/// One v100-unlock at a time. The lock is held until the file is dropped.
pub fn lock() -> Result<File, Outcome> {
    lock_at(&[LOCK_PATH])
}

/// [`lock`] on the first of `paths` that can be opened.
fn lock_at(paths: &[&str]) -> Result<File, Outcome> {
    let file = paths
        .iter()
        .find_map(|path| {
            OpenOptions::new()
                .create(true)
                .write(true)
                .truncate(false)
                .mode(0o644)
                .custom_flags(libc::O_NOFOLLOW)
                .open(path)
                .ok()
        })
        .ok_or_else(|| {
            refused(format!(
                "cannot create the lock file {}",
                paths.join(" or ")
            ))
        })?;
    // SAFETY: flock on a descriptor this function owns.
    if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0 {
        return Err(refused("another v100-unlock is already running"));
    }
    Ok(file)
}

/// The cards the user named, in PCI order; every card if none was named.
pub fn select(cards: Vec<Card>, wanted: &[Bdf]) -> Result<Vec<Card>, Outcome> {
    if let Some(missing) = wanted.iter().find(|w| !cards.iter().any(|c| &c.bdf == *w)) {
        return Err(refused(format!("no Tesla V100 at {missing}")));
    }
    Ok(cards
        .into_iter()
        .filter(|card| wanted.is_empty() || wanted.contains(&card.bdf))
        .collect())
}

/// Every selected Tesla V100 in this machine, with its BAR0 mapped.
pub fn open_cards(wanted: &[Bdf], access: Access) -> Result<Vec<AnyUnit<'static>>, Outcome> {
    let sysfs = Path::new(SYSFS_PCI);
    let found = discover(sysfs).map_err(|e| refused(e.to_string()))?;
    select(found, wanted)?
        .into_iter()
        .map(|card| {
            let bar = MmapBar0::open(&card.resource0(sysfs), access)
                .map_err(|e| refused(format!("{}: {e}", card.bdf.short())))?;
            let regs = Regs::new(Box::new(bar) as Box<dyn Bar0>, access);
            Ok(Unit { card, regs })
        })
        .collect()
}

pub fn refused(reason: impl Into<String>) -> Outcome {
    Outcome::new(Kind::Refused, reason)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_lock(tag: &str) -> String {
        let path =
            std::env::temp_dir().join(format!("v100-unlock-{tag}-{}.lock", std::process::id()));
        let _ = std::fs::remove_file(&path);
        path.to_string_lossy().into_owned()
    }

    #[test]
    fn only_one_run_holds_the_lock() {
        let path = temp_lock("lock");
        let first = lock_at(&[&path]).expect("first lock");
        let second = lock_at(&[&path]).expect_err("second lock while the first is held");
        assert_eq!(second.summary, "another v100-unlock is already running");
        drop(first);
        assert!(
            lock_at(&[&path]).is_ok(),
            "free again once the holder is gone"
        );
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn the_lock_never_follows_a_symlink() {
        let path = temp_lock("symlink");
        let target = format!("{path}.target");
        std::fs::write(&target, "do not touch").unwrap();
        std::os::unix::fs::symlink(&target, &path).unwrap();
        assert!(lock_at(&[&path]).is_err());
        assert_eq!(std::fs::read_to_string(&target).unwrap(), "do not touch");
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_file(&target);
    }

    #[test]
    fn the_lock_falls_back_to_the_next_path() {
        let path = temp_lock("fallback");
        assert!(lock_at(&["/nonexistent/dir/v100-unlock.lock", &path]).is_ok());
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn selecting_cards_keeps_pci_order_and_refuses_strangers() {
        let model = crate::card::model_for(0x1db1).unwrap();
        let card = |bdf: &str| Card {
            bdf: bdf.parse().unwrap(),
            model,
        };
        let all = vec![
            card("0000:04:00.0"),
            card("0000:06:00.0"),
            card("0000:08:00.0"),
        ];
        let pick = |wanted: &[&str]| {
            let wanted: Vec<Bdf> = wanted.iter().map(|w| w.parse().unwrap()).collect();
            select(all.clone(), &wanted).map(|cards| {
                cards
                    .iter()
                    .map(|c| c.bdf.short().to_string())
                    .collect::<Vec<_>>()
            })
        };
        assert_eq!(pick(&[]).unwrap(), ["04:00.0", "06:00.0", "08:00.0"]);
        assert_eq!(
            pick(&["0000:08:00.0", "0000:04:00.0"]).unwrap(),
            ["04:00.0", "08:00.0"]
        );
        assert_eq!(
            pick(&["0000:05:00.0"]).unwrap_err().summary,
            "no Tesla V100 at 0000:05:00.0"
        );
    }
}
