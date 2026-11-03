"""Read one top-level regular DEB from a vendor ZIP; never extract paths."""
import json
import os
import stat
import sys
import zipfile

MAX_BYTES = 1024 * 1024 * 1024


def extract(archive, destination):
    if os.path.getsize(archive) > MAX_BYTES:
        raise ValueError("ZIP exceeds size limit")
    with zipfile.ZipFile(archive) as source:
        members = source.infolist()
        if len(members) > 1024:
            raise ValueError("Too many ZIP members")
        debs = [m for m in members if m.filename.endswith(".deb")]
        if len(debs) != 1:
            raise ValueError("ZIP must contain exactly one DEB")
        member = debs[0]
        name = member.filename
        mode = member.external_attr >> 16
        if (not name or name.startswith(".") or any(c in name for c in "/\\\0")
                or member.is_dir() or member.flag_bits & 1
                or stat.S_IFMT(mode) not in (0, stat.S_IFREG)
                or member.compress_type not in (zipfile.ZIP_STORED, zipfile.ZIP_DEFLATED)
                or not 0 < member.file_size <= MAX_BYTES):
            raise ValueError("Unsafe or unsupported DEB member")
        try:
            with source.open(member) as input_file, open(destination, "xb") as output:
                size = 0
                while chunk := input_file.read(65536):
                    size += len(chunk)
                    if size > member.file_size:
                        raise ValueError("DEB exceeds declared size")
                    output.write(chunk)
                if size != member.file_size:
                    raise ValueError("DEB size mismatch")
        except Exception:
            # Do not remove an existing caller-owned output file.
            if 'output' in locals():
                os.unlink(destination)
            raise
        return {"member": name}


if __name__ == "__main__":
    print(json.dumps(extract(sys.argv[1], sys.argv[2])))
