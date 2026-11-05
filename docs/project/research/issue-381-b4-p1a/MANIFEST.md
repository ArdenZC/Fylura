# Evidence hash manifest

`evidence/final/MANIFEST.sha256` lists SHA-256 hashes for every file under the isolated harness and this evidence package, including compressed raw streams. The manifest excludes itself because a file cannot contain its own digest.

Verify it from the repository root with:

```sh
sha256sum -c docs/project/research/issue-381-b4-p1a/evidence/final/MANIFEST.sha256
```
