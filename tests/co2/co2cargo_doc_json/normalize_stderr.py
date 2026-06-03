import sys

with open(sys.argv[1], encoding="utf-8") as f:
    text = f.read()

# Strip the Documenting line (contains absolute path) and
# the Finished line (contains timing info).
lines = text.splitlines()
filtered = []
for line in lines:
    stripped = line.strip()
    if stripped.startswith("Documenting ") or stripped.startswith("Finished "):
        continue
    filtered.append(line)

sys.stdout.write("\n".join(filtered))
sys.stdout.write("\n")
