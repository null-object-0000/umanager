"""Extract only regular data files from a pinned GitHub Pages artifact."""
import pathlib
import sys
import tarfile

archive, destination = map(pathlib.Path, sys.argv[1:])
destination = destination.resolve()
destination.mkdir(parents=True, exist_ok=True)
if any(destination.iterdir()):
    raise RuntimeError("Extraction destination must be empty")
with tarfile.open(archive) as source:
    members = source.getmembers()
    seen = set()
    total = 0
    for member in members:
        relative = pathlib.PurePosixPath(member.name)
        if relative.is_absolute() or ".." in relative.parts or "\\" in member.name or ":" in member.name:
            raise RuntimeError("Unsafe archive path")
        target = (destination / str(relative)).resolve()
        if not target.is_relative_to(destination) or not (member.isfile() or member.isdir()):
            raise RuntimeError("Archive links and special files are forbidden")
        if member.isfile():
            key = str(relative).casefold()
            if key in seen:
                raise RuntimeError("Duplicate archive file")
            seen.add(key)
            total += member.size
            if member.size > 25 * 1024 * 1024 or total > 100 * 1024 * 1024:
                raise RuntimeError("Unexpectedly large publication")
    for member in members:
        target = destination / str(pathlib.PurePosixPath(member.name))
        if member.isdir():
            target.mkdir(parents=True, exist_ok=True)
        else:
            target.parent.mkdir(parents=True, exist_ok=True)
            with source.extractfile(member) as incoming, target.open("wb") as outgoing:
                outgoing.write(incoming.read())
print(f"Extracted {len(seen)} regular publication files")
