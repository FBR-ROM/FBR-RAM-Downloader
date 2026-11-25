//! How a run ended, named for the state it leaves the hardware in.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// Every card is at the target clock.
    Done,
    /// Nothing was written; every card is as it was.
    Refused,
    /// Something failed or was cancelled; every card is back where it started.
    RolledBack,
    /// A card may be at an unknown clock; power-cycle the host.
    Unknown,
}

impl Kind {
    /// Process exit code: 0 at target, 2 untouched, 3 put back, 4 unknown.
    pub fn exit_code(self) -> i32 {
        match self {
            Kind::Done => 0,
            Kind::Refused => 2,
            Kind::RolledBack => 3,
            Kind::Unknown => 4,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Kind::Done => "ok",
            Kind::Refused => "refused",
            Kind::RolledBack => "rolled-back",
            Kind::Unknown => "unknown",
        }
    }

    pub fn parse(word: &str) -> Option<Kind> {
        [Kind::Done, Kind::Refused, Kind::RolledBack, Kind::Unknown]
            .into_iter()
            .find(|k| k.as_str() == word)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Outcome {
    pub kind: Kind,
    pub summary: String,
}

impl Outcome {
    pub fn new(kind: Kind, summary: impl Into<String>) -> Self {
        Self {
            kind,
            summary: summary.into(),
        }
    }
}
