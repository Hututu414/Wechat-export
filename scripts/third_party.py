"""Collect notices from the exact locally cached Cargo graph; does not access the network."""
import json
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
metadata = json.loads((ROOT / ".local/metadata.json").read_text(encoding="utf-8-sig"))
upstream_file = ROOT / "licenses/upstream/sources.json"
upstream = json.loads(upstream_file.read_text(encoding="utf-8")) if upstream_file.exists() else []
nodes = {node["id"]: node for node in metadata["resolve"]["nodes"]}
packages = {package["id"]: package for package in metadata["packages"]}
seen = set()
pending = [metadata["resolve"]["root"]]
while pending:
    current = pending.pop()
    if current in seen:
        continue
    seen.add(current)
    pending.extend(dep["pkg"] for dep in nodes[current]["deps"]
                   if any(kind["kind"] != "dev" for kind in dep["dep_kinds"]))

output = ["Cargo dependency notices (Windows x64, locked dependency graph)",
          "Includes build dependencies; target-specific non-Windows dependencies are excluded.",
          "Each bundled crate retains its own license. OS fonts are loaded locally, not redistributed."]
missing = []
count = 0
for package in sorted((packages[key] for key in seen if packages[key]["source"]),
                      key=lambda p: (p["name"], p["version"])):
    count += 1
    root = Path(package["manifest_path"]).parent
    output.extend(["\n" + "=" * 72, f'{package["name"]} {package["version"]}',
                   f'License: {package["license"]}', f'Source: {package["repository"] or package["source"]}'])
    files = [path for path in root.iterdir() if path.is_file()
             and path.name.upper().startswith(("LICENSE", "LICENCE", "COPYING", "NOTICE", "UNLICENSE"))]
    for subdir in [root / "licenses", root / "LICENSES"]:
        if subdir.is_dir():
            files.extend(path for path in subdir.rglob("*") if path.is_file())
    for relative in ["zstd/LICENSE", "zstd/COPYING"]:
        if (root / relative).is_file():
            files.append(root / relative)
    if package["name"] == "epaint_default_fonts":
        files.extend((root / "fonts").glob("*.txt"))
    if package.get("license_file") and (root / package["license_file"]).is_file():
        files.append(root / package["license_file"])
    files = sorted(set(files))
    supplement = [record for record in upstream if record["package"] == package["name"] and record["version"] == package["version"]]
    if not files and not supplement:
        missing.append(package["name"])
    for path in files:
        output.extend([f"\n--- {path.relative_to(root)} ---", path.read_text(encoding="utf-8", errors="replace")])
    for record in supplement:
        for source in record["sources"]:
            output.extend([f'\n--- {source["url"]} ---', (ROOT / source["file"]).read_text(encoding="utf-8")])
(ROOT / "licenses/dependencies.txt").write_text("\n".join(output), encoding="utf-8")
print(f"packages={count}; packages_without_license_files={missing}")
