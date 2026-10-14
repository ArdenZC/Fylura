#!/usr/bin/env python3
"""Independent fixture-string BM25 oracle; deliberately does not load SQLite."""

from __future__ import annotations

import argparse
import gzip
import hashlib
import json
import math
import struct
from pathlib import Path


def fixture_documents(count: int) -> list[dict[str, object]]:
    result = []
    for index in range(count):
        bucket = index % 10
        roots: list[str] = []
        if bucket <= 7:
            roots.append("root-a-healthy")
        if bucket in (2, 3, 7, 8):
            roots.append("root-b-degraded-retained")
        if bucket in (8, 9):
            roots.append("root-d-disabled")
        suffix = " rareanchor" if index % 2500 == 0 else ""
        result.append(
            {
                "uid": f"source-{index:06}",
                "observation_revision": 1,
                "name": f"common frozen-file-{index:06}{suffix}",
                "path": f"/synthetic/root-{index % 4}/segment/file-{index:06}.dat",
                "mtime": 1_700_000_000 + ((index * 7_919) % 1_000_000),
                "roots": roots,
                "effective": "root-a-healthy" in roots
                or "root-b-degraded-retained" in roots,
            }
        )
    return result


def corpus_digest(docs: list[dict[str, object]]) -> str:
    digest = hashlib.sha256()
    for doc in docs:
        digest.update(str(doc["uid"]).encode())
        digest.update(b"\0")
        digest.update(struct.pack("<I", int(doc["observation_revision"])))
        digest.update(b"\0")
        digest.update(str(doc["name"]).encode())
        digest.update(b"\0")
        digest.update(str(doc["path"]).encode())
        digest.update(b"\0")
        digest.update(struct.pack("<q", int(doc["mtime"])))
        digest.update(b"\0")
        for root in doc["roots"]:
            digest.update(str(root).encode())
            digest.update(b"\0")
        digest.update(bytes((int(bool(doc["effective"])), 0xFF)))
    return digest.hexdigest()


def fts5_length(text: str) -> int:
    # All benchmark text is ASCII; the pinned trigram tokenizer emits len-2 tokens.
    return max(0, len(text) - 2)


def score_rows(docs: list[dict[str, object]], query: str, root: str | None):
    effective = [doc for doc in docs if doc["effective"]]
    hits = [doc for doc in effective if query in str(doc["name"]) or query in str(doc["path"])]
    total = len(effective)
    average_length = sum(
        fts5_length(str(doc["name"])) + fts5_length(str(doc["path"]))
        for doc in effective
    ) / total if total else 0.0
    df = len(hits)
    if not total or not df:
        return []
    idf_ratio = (total - df + 0.5) / (df + 0.5)
    idf = math.log(idf_ratio)
    if idf <= 0.0:
        idf = 1.0e-6
    weighted: list[tuple[float, dict[str, object]]] = []
    for doc in hits:
        name = str(doc["name"])
        path = str(doc["path"])
        frequency = name.count(query) * 6.0 + path.count(query) * 1.5
        length = fts5_length(name) + fts5_length(path)
        denominator = frequency + 1.2 * (0.25 + 0.75 * length / average_length)
        score = -(idf * (frequency * 2.2 / denominator))
        weighted.append((score, doc))
    if root is not None:
        weighted = [
            item for item in weighted if root in item[1]["roots"]
        ]
    weighted.sort(
        key=lambda item: (
            item[0],
            -int(item[1]["mtime"]),
            str(item[1]["name"]).lower(),
            str(item[1]["uid"]),
        )
    )
    return [
        {
            "uid": str(doc["uid"]),
            "score_bits": struct.pack(">d", score).hex(),
            "mtime": int(doc["mtime"]),
            "name": str(doc["name"]),
            "path": str(doc["path"]),
        }
        for score, doc in weighted
    ]


def open_jsonl(path: Path):
    if path.suffix == ".gz":
        return gzip.open(path, "rt", encoding="utf-8")
    return path.open("rt", encoding="utf-8")


def check_query(record: dict[str, object]) -> dict[str, object]:
    details = record["details"]
    assert isinstance(details, dict)
    count = int(details["generated_document_count"])
    docs = fixture_documents(count)
    actual_corpus = str(details["corpus_sha256"])
    calculated_corpus = corpus_digest(docs)
    query = str(details["query"])
    root = details["root_filter"]
    expected = score_rows(docs, query, None if root is None else str(root))
    actual = details["rows"]
    mismatches = []
    if len(expected) != len(actual):
        mismatches.append(
            {"kind": "row_count", "expected": len(expected), "actual": len(actual)}
        )
    for index, (want, got) in enumerate(zip(expected, actual)):
        if want != got:
            mismatches.append(
                {"kind": "row", "index": index, "expected": want, "actual": got}
            )
            if len(mismatches) >= 8:
                break
    return {
        "record_type": "oracle_check",
        "case_id": "P1A-T03",
        "status": "PASS"
        if not mismatches and actual_corpus == calculated_corpus
        else "FAIL",
        "generated_document_count": count,
        "query": query,
        "root_filter": root,
        "expected_rows": len(expected),
        "actual_rows": len(actual),
        "corpus_sha256_expected": calculated_corpus,
        "corpus_sha256_actual": actual_corpus,
        "full_sorted_uid_sha256_expected": hashlib.sha256(
            "\n".join(row["uid"] for row in expected).encode()
        ).hexdigest(),
        "full_sorted_uid_sha256_actual": hashlib.sha256(
            "\n".join(str(row["uid"]) for row in actual).encode()
        ).hexdigest(),
        "exact_float_bits_and_order": not mismatches,
        "mismatches": mismatches,
    }


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("native_jsonl", type=Path)
    parser.add_argument("output_jsonl", type=Path)
    args = parser.parse_args()
    args.output_jsonl.parent.mkdir(parents=True, exist_ok=True)
    checks = []
    with open_jsonl(args.native_jsonl) as stream:
        for line in stream:
            record = json.loads(line)
            if record.get("record_type") == "query_snapshot":
                checks.append(check_query(record))
    with args.output_jsonl.open("wt", encoding="utf-8", newline="\n") as output:
        for check in checks:
            output.write(json.dumps(check, sort_keys=True, separators=(",", ":")) + "\n")
    failed = [check for check in checks if check["status"] != "PASS"]
    summary = {
        "oracle": "independent Python fixture-string BM25; does not import or invoke SQLite",
        "query_snapshots_checked": len(checks),
        "passed": len(checks) - len(failed),
        "failed": len(failed),
        "exact_float_bits": True,
    }
    print(json.dumps(summary, sort_keys=True))
    return 1 if failed else 0


if __name__ == "__main__":
    raise SystemExit(main())
