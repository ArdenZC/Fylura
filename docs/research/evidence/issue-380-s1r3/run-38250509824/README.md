# S1R3 Run 38250509824 Evidence

`artifact.zip` is the byte-for-byte GitHub Actions artifact ZIP. Its GitHub SHA-256 is recorded in `run-metadata.json`. The JSON/text files beside it are LF-normalized convenience copies; the original member bytes, including the original `SHA256SUMS`, remain inside `artifact.zip`.

The internal artifact `SHA256SUMS` was checked against all 10 listed members with no mismatches. `workflow-run.txt` is the complete Actions log captured through `gh run view --log`.

The Run stopped before NSIS was invoked because the Runner rejected its task root as a working directory. Therefore this evidence contains no Probe compilation, generated uninstaller, Abort process, or FileWrite trace.
