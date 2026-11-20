#!/usr/bin/env python3
"""Generate src/model_data.rs from fbrnanologo.obj (embedded, self-contained)."""
import sys

OBJ = "/home/arikami/Downloads/fbrnanologo/fbrnanologo.obj"
OUT = "src/model_data.rs"

verts = []
faces = []
max_face = 0
with open(OBJ) as f:
    for line in f:
        line = line.strip()
        if line.startswith("v "):
            p = line.split()
            verts.append((float(p[1]), float(p[2]), float(p[3])))
        elif line.startswith("f "):
            idx = []
            for tok in line.split()[1:]:
                idx.append(int(tok.split("//")[0].split("/")[0]) - 1)
            max_face = max(max_face, len(idx))
            faces.append(idx)

print(f"verts: {len(verts)}, faces: {len(faces)}, max face verts: {max_face}", file=sys.stderr)

# Triangulate (fan) any polygon faces.
tris = []
for face in faces:
    for i in range(1, len(face) - 1):
        tris.append((face[0], face[i], face[i + 1]))
print(f"triangles: {len(tris)}", file=sys.stderr)

with open(OUT, "w") as f:
    f.write("// Auto-generated from fbrnanologo.obj. Do not edit by hand.\n")
    f.write("// 56 vertices, triangulated faces (0-based indices).\n\n")
    f.write("pub const VERTS: &[(f32, f32, f32)] = &[\n")
    for (x, y, z) in verts:
        f.write(f"    ({x:.6f}, {y:.6f}, {z:.6f}),\n")
    f.write("];\n\n")
    f.write("pub const FACES: &[(usize, usize, usize)] = &[\n")
    for (a, b, c) in tris:
        f.write(f"    ({a}, {b}, {c}),\n")
    f.write("];\n")

print(f"wrote {OUT}", file=sys.stderr)
