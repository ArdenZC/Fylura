use serde::Serialize;
use sha2::{Digest, Sha256};

#[derive(Clone, Debug, Serialize)]
pub struct Document {
    pub uid: String,
    pub observation_revision: u32,
    pub name: String,
    pub path: String,
    pub mtime: i64,
    pub root_claims: Vec<String>,
    pub effective: bool,
}

pub fn documents(count: usize) -> Vec<Document> {
    (0..count)
        .map(|i| {
            let bucket = i % 10;
            let mut root_claims = Vec::new();
            if bucket <= 7 {
                root_claims.push("root-a-healthy".to_string());
            }
            if matches!(bucket, 2 | 3 | 7 | 8) {
                root_claims.push("root-b-degraded-retained".to_string());
            }
            if matches!(bucket, 8 | 9) {
                root_claims.push("root-d-disabled".to_string());
            }
            let rare = i.is_multiple_of(2500);
            let name = if rare {
                format!("common frozen-file-{i:06} rareanchor")
            } else {
                format!("common frozen-file-{i:06}")
            };
            let path = format!("/synthetic/root-{}/segment/file-{i:06}.dat", i % 4);
            let effective = root_claims
                .iter()
                .any(|root| root == "root-a-healthy" || root == "root-b-degraded-retained");
            Document {
                uid: format!("source-{i:06}"),
                observation_revision: 1,
                name,
                path,
                mtime: 1_700_000_000 + ((i as i64 * 7_919) % 1_000_000),
                root_claims,
                effective,
            }
        })
        .collect()
}

pub fn corpus_sha256(docs: &[Document]) -> String {
    let mut hash = Sha256::new();
    for doc in docs {
        hash.update(doc.uid.as_bytes());
        hash.update([0]);
        hash.update(doc.observation_revision.to_le_bytes());
        hash.update([0]);
        hash.update(doc.name.as_bytes());
        hash.update([0]);
        hash.update(doc.path.as_bytes());
        hash.update([0]);
        hash.update(doc.mtime.to_le_bytes());
        hash.update([0]);
        for claim in &doc.root_claims {
            hash.update(claim.as_bytes());
            hash.update([0]);
        }
        hash.update([u8::from(doc.effective), 0xff]);
    }
    format!("{:x}", hash.finalize())
}

pub fn file_sha256(path: &std::path::Path) -> std::io::Result<(String, u64)> {
    use std::io::Read;
    let mut file = std::fs::File::open(path)?;
    let mut hash = Sha256::new();
    let mut bytes = 0_u64;
    let mut chunk = [0_u8; 64 * 1024];
    loop {
        let read = file.read(&mut chunk)?;
        if read == 0 {
            break;
        }
        hash.update(&chunk[..read]);
        bytes += read as u64;
    }
    Ok((format!("{:x}", hash.finalize()), bytes))
}
