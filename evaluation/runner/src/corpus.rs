use arrayvec::ArrayVec;
use serde::{
    Deserialize, Deserializer, Serialize,
    de::{self, MapAccess, Visitor},
};
use sha2::{Digest, Sha256};
use std::{
    fs::{self, File},
    io::{self, Read},
    path::{Component, Path, PathBuf},
};
use thiserror::Error;
use unicode_general_category::{GeneralCategory, get_general_category};
use unicode_normalization::UnicodeNormalization;

pub static MAX_MANIFEST_BYTES: u64 = 2 * 1024 * 1024;
static MAX_REFERENCE_BYTES: u64 = 2 * 1024 * 1024;

#[derive(Debug, Error)]
pub enum CorpusError {
    #[error("corpus I/O failed")]
    Io(#[from] io::Error),
    #[error("invalid corpus JSON")]
    Json(#[from] serde_json::Error),
    #[error("corpus admission failed: {0}")]
    Invalid(&'static str),
    #[error("asset size or SHA-256 differs: {0}")]
    Hash(PathBuf),
}

#[derive(Debug, Deserialize)]
pub struct Asset {
    pub path: PathBuf,
    pub sha256: String,
    pub bytes: u64,
}
#[derive(Debug, Deserialize)]
pub struct English {
    pub status: String,
    pub reference_letters: usize,
}
#[derive(Debug, Deserialize)]
pub struct Audit {
    pub status: String,
    pub input_reference_alignment: bool,
    pub reference_covers_task: bool,
    pub reading_order_reviewed: bool,
    pub english_reviewed: bool,
    pub transcription_policy_reviewed: bool,
}
#[derive(Debug, Deserialize)]
pub struct Case {
    pub id: String,
    pub source_id: String,
    pub original_page_id: String,
    pub document_cluster: String,
    pub writer_cluster: Option<String>,
    pub cohort: String,
    pub task: String,
    pub language: String,
    pub script: String,
    pub tesseract_language: String,
    pub word_tokenizer: String,
    pub english: English,
    pub domain: String,
    pub appearance: String,
    pub reference_quality: String,
    pub input: Asset,
    pub reference: Asset,
    pub annotation: Option<Asset>,
    pub width: u32,
    pub height: u32,
    pub input_mime: String,
    pub audit: Audit,
}
#[derive(Debug, Deserialize)]
pub struct Corpus {
    pub schema_version: u32,
    pub corpus_id: String,
    pub status: String,
    pub selection_seed: u64,
    pub expected_primary_pages: usize,
    #[serde(deserialize_with = "sources")]
    pub sources: ArrayVec<(String, Source), 32>,
    pub cases: ArrayVec<Case, 128>,
}

#[derive(Debug, Deserialize)]
pub struct Source {
    pub title: String,
    pub url: String,
    pub revision: String,
    pub license: String,
    pub license_evidence_url: String,
    pub license_scope: String,
    pub attribution: String,
    pub admission: String,
}

fn sources<'de, D: Deserializer<'de>>(
    deserializer: D,
) -> Result<ArrayVec<(String, Source), 32>, D::Error> {
    struct Sources;
    impl<'de> Visitor<'de> for Sources {
        type Value = ArrayVec<(String, Source), 32>;
        fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            formatter.write_str("a map of at most 32 uniquely named corpus sources")
        }
        fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Self::Value, A::Error> {
            let mut sources: Self::Value = ArrayVec::new();
            while let Some((id, source)) = map.next_entry::<String, Source>()? {
                if id.trim().is_empty() || sources.iter().any(|(known, _)| known == &id) {
                    return Err(de::Error::custom("empty or duplicate source identifier"));
                }
                sources
                    .try_push((id, source))
                    .map_err(|_| de::Error::custom("more than 32 corpus sources"))?;
            }
            Ok(sources)
        }
    }
    deserializer.deserialize_map(Sources)
}

fn validate_source(source: &Source) -> Result<(), CorpusError> {
    if source.admission != "accepted"
        || [
            source.title.as_str(),
            source.revision.as_str(),
            source.license.as_str(),
            source.license_scope.as_str(),
            source.attribution.as_str(),
        ]
        .iter()
        .any(|value| value.trim().is_empty())
    {
        return Err(CorpusError::Invalid(
            "source lacks accepted, explicit revision/rights/attribution metadata",
        ));
    }
    for value in [&source.url, &source.license_evidence_url] {
        let url = reqwest::Url::parse(value)
            .map_err(|_| CorpusError::Invalid("source or license evidence URL is invalid"))?;
        if url.scheme() != "https"
            || url.host_str().is_none()
            || !url.username().is_empty()
            || url.password().is_some()
        {
            return Err(CorpusError::Invalid(
                "source and license evidence require public HTTPS URLs without credentials",
            ));
        }
    }
    Ok(())
}

fn letters(text: &str) -> usize {
    text.nfc()
        .filter(|&character| {
            matches!(
                get_general_category(character),
                GeneralCategory::UppercaseLetter
                    | GeneralCategory::LowercaseLetter
                    | GeneralCategory::TitlecaseLetter
                    | GeneralCategory::ModifierLetter
                    | GeneralCategory::OtherLetter
            )
        })
        .count()
}

fn english_letters(english: &English, letters: usize) -> Result<usize, CorpusError> {
    if english.reference_letters > letters {
        return Err(CorpusError::Invalid(
            "declared English letters exceed the reference's NFC Letter count",
        ));
    }
    match english.status.as_str() {
        "absent" if english.reference_letters == 0 => Ok(0),
        "present" if english.reference_letters != 0 => Ok(english.reference_letters),
        "unknown" => Ok(letters),
        _ => Err(CorpusError::Invalid(
            "English status/count must be absent/zero, present/nonzero, or explicitly unknown",
        )),
    }
}
#[derive(Debug, Deserialize)]
pub struct Download {
    #[serde(flatten)]
    pub asset: Asset,
    pub url: String,
    pub max_bytes: u64,
}
#[derive(Debug, Deserialize)]
pub struct Downloads {
    pub schema_version: u32,
    pub max_total_bytes: u64,
    pub assets: ArrayVec<Download, 256>,
}

#[derive(Debug, Serialize)]
pub struct Admission {
    pub corpus_id: String,
    pub cases: usize,
    pub primary_pages: usize,
    pub primary_document_clusters: usize,
    pub english_pages_conservative: usize,
    pub english_clusters_conservative: usize,
    pub english_reference_letters: usize,
    pub reference_letters: usize,
    pub files_verified: usize,
}

pub fn read_json<T: for<'de> Deserialize<'de>>(path: &Path) -> Result<T, CorpusError> {
    let file = File::open(path)?;
    if file.metadata()?.len() > MAX_MANIFEST_BYTES {
        return Err(CorpusError::Invalid("manifest is oversized"));
    }
    Ok(serde_json::from_reader(io::BufReader::new(
        file.take(MAX_MANIFEST_BYTES + 1),
    ))?)
}

/// Hash the exact bounded bytes consumed by JSON parsing, including trailing
/// whitespace, so the declared corpus identity cannot conceal a changed manifest.
pub fn read_json_hashed<T: for<'de> Deserialize<'de>>(
    path: &Path,
) -> Result<(T, String, u64), CorpusError> {
    struct DigestReader {
        source: io::Take<File>,
        digest: Sha256,
        bytes: u64,
    }
    impl Read for DigestReader {
        fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
            let read = self.source.read(buffer)?;
            self.digest.update(&buffer[..read]);
            self.bytes += read as u64;
            Ok(read)
        }
    }
    let file = File::open(path)?;
    let metadata = file.metadata()?;
    if !metadata.is_file() || metadata.len() > MAX_MANIFEST_BYTES {
        return Err(CorpusError::Invalid(
            "manifest must be a bounded regular file",
        ));
    }
    let mut reader = DigestReader {
        source: file.take(MAX_MANIFEST_BYTES + 1),
        digest: Sha256::new(),
        bytes: 0,
    };
    let value = serde_json::from_reader(io::BufReader::new(&mut reader))?;
    if reader.bytes > MAX_MANIFEST_BYTES {
        return Err(CorpusError::Invalid("manifest grew beyond its byte bound"));
    }
    Ok((
        value,
        format!("{:x}", reader.digest.finalize()),
        reader.bytes,
    ))
}

pub fn safe_path(root: &Path, relative: &Path) -> Result<PathBuf, CorpusError> {
    if relative.as_os_str().is_empty()
        || relative
            .components()
            .any(|c| !matches!(c, Component::Normal(_)))
    {
        return Err(CorpusError::Invalid(
            "asset path must be relative and contain only normal components",
        ));
    }
    let mut path = PathBuf::from(root);
    for component in relative.components() {
        path.push(component);
        match fs::symlink_metadata(&path) {
            Ok(metadata) if metadata.file_type().is_symlink() => {
                return Err(CorpusError::Invalid("symlink in corpus asset path"));
            }
            Ok(_) => (),
            Err(error) if error.kind() == io::ErrorKind::NotFound => (),
            Err(error) => return Err(error.into()),
        }
    }
    Ok(path)
}

pub fn verify_asset(root: &Path, asset: &Asset) -> Result<PathBuf, CorpusError> {
    let path = safe_path(root, &asset.path)?;
    let mut file = File::open(&path)?;
    let metadata = file.metadata()?;
    if !metadata.is_file() || metadata.len() != asset.bytes {
        return Err(CorpusError::Hash(path));
    }
    let mut hash = Sha256::new();
    let mut buffer = [0_u8; 65_536];
    let mut count = 0_u64;
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        count += read as u64;
        if count > asset.bytes {
            return Err(CorpusError::Hash(path));
        }
        hash.update(&buffer[..read]);
    }
    let actual = format!("{:x}", hash.finalize());
    if count != asset.bytes || actual != asset.sha256 {
        return Err(CorpusError::Hash(path));
    }
    Ok(path)
}

impl Corpus {
    pub fn validate(&self, root: &Path) -> Result<Admission, CorpusError> {
        if self.schema_version != 1
            || self.status != "frozen"
            || self.cases.is_empty()
            || self.corpus_id.trim().is_empty()
        {
            return Err(CorpusError::Invalid("requires nonempty frozen schema 1"));
        }
        let mut ids: ArrayVec<&str, 128> = ArrayVec::new();
        let mut pages: ArrayVec<&str, 128> = ArrayVec::new();
        let mut english_pages: ArrayVec<&str, 128> = ArrayVec::new();
        let mut clusters: ArrayVec<&str, 128> = ArrayVec::new();
        let mut english_clusters: ArrayVec<&str, 128> = ArrayVec::new();
        let mut reference_letters = 0;
        let mut conservative_english_letters = 0;
        let mut reference = String::with_capacity(131_072);
        for case in &self.cases {
            if case.id.is_empty()
                || case.id.len() > 128
                || ids.contains(&case.id.as_str())
                || !case
                    .id
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
            {
                return Err(CorpusError::Invalid(
                    "case id must be a unique bounded safe filename identifier",
                ));
            }
            ids.push(&case.id);
            if [
                case.original_page_id.as_str(),
                case.document_cluster.as_str(),
                case.language.as_str(),
                case.script.as_str(),
                case.domain.as_str(),
                case.appearance.as_str(),
                case.reference_quality.as_str(),
                case.task.as_str(),
                case.tesseract_language.as_str(),
                case.input_mime.as_str(),
            ]
            .iter()
            .any(|value| value.trim().is_empty())
                || case
                    .writer_cluster
                    .as_deref()
                    .is_some_and(|value| value.trim().is_empty())
                || case.width == 0
                || case.height == 0
            {
                return Err(CorpusError::Invalid(
                    "case identity, analysis strata, tokenizer context and dimensions must be explicit",
                ));
            }
            if !matches!(case.word_tokenizer.as_str(), "whitespace" | "none") {
                return Err(CorpusError::Invalid(
                    "word tokenizer must explicitly be whitespace or none",
                ));
            }
            let (_, source) = self
                .sources
                .iter()
                .find(|(id, _)| id == &case.source_id)
                .ok_or(CorpusError::Invalid("case references an unknown source"))?;
            validate_source(source)?;
            if self.cases.iter().any(|other| {
                other.original_page_id == case.original_page_id
                    && other.cohort == case.cohort
                    && other.document_cluster != case.document_cluster
            }) {
                return Err(CorpusError::Invalid(
                    "shared original-page groups must remain in one document cluster",
                ));
            }
            if case.audit.status != "reviewed"
                || !case.audit.input_reference_alignment
                || !case.audit.reference_covers_task
                || !case.audit.reading_order_reviewed
                || !case.audit.english_reviewed
                || !case.audit.transcription_policy_reviewed
            {
                return Err(CorpusError::Invalid(
                    "case lacks a complete task/reference audit",
                ));
            }
            verify_asset(root, &case.input)?;
            if case.reference.bytes > MAX_REFERENCE_BYTES {
                return Err(CorpusError::Invalid(
                    "reference text exceeds its byte bound",
                ));
            }
            let reference_path = verify_asset(root, &case.reference)?;
            reference.clear();
            File::open(&reference_path)?
                .take(MAX_REFERENCE_BYTES + 1)
                .read_to_string(&mut reference)?;
            if reference.len() as u64 != case.reference.bytes
                || format!("{:x}", Sha256::digest(reference.as_bytes())) != case.reference.sha256
            {
                return Err(CorpusError::Hash(reference_path));
            }
            let letters = letters(&reference);
            let case_english_letters = english_letters(&case.english, letters)?;
            if case.cohort == "primary" {
                reference_letters += letters;
                if pages.contains(&case.original_page_id.as_str()) {
                    return Err(CorpusError::Invalid(
                        "primary cases must identify distinct original pages",
                    ));
                }
                pages.push(&case.original_page_id);
                if !clusters.contains(&case.document_cluster.as_str()) {
                    clusters.push(&case.document_cluster);
                }
                if case.english.status != "absent" {
                    if !english_pages.contains(&case.original_page_id.as_str()) {
                        english_pages.push(&case.original_page_id);
                    }
                    if !english_clusters.contains(&case.document_cluster.as_str()) {
                        english_clusters.push(&case.document_cluster);
                    }
                    conservative_english_letters += case_english_letters;
                }
            } else if case.cohort != "challenge" {
                return Err(CorpusError::Invalid("unknown cohort"));
            }
        }
        if pages.len() != self.expected_primary_pages {
            return Err(CorpusError::Invalid("primary page count changed"));
        }
        if reference_letters == 0
            || english_pages.len() * 10 >= pages.len()
            || english_clusters.len() * 10 >= clusters.len()
            || conservative_english_letters * 10 >= reference_letters
        {
            return Err(CorpusError::Invalid(
                "English must be strictly below 10% of pages, clusters and reference letters",
            ));
        }
        Ok(Admission {
            corpus_id: String::from(&self.corpus_id),
            cases: self.cases.len(),
            primary_pages: pages.len(),
            primary_document_clusters: clusters.len(),
            english_pages_conservative: english_pages.len(),
            english_clusters_conservative: english_clusters.len(),
            english_reference_letters: conservative_english_letters,
            reference_letters,
            files_verified: self.cases.len() * 2,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rejects_escaping_and_absolute_paths() {
        for path in ["../secret", "/tmp/a", "a/../../b", ""] {
            assert!(safe_path(Path::new("/tmp"), Path::new(path)).is_err());
        }
        assert!(safe_path(Path::new("/tmp"), Path::new("a/b.png")).is_ok());
    }

    #[test]
    fn english_denominator_is_nfc_letter_category_and_unknown_is_conservative() {
        assert_eq!(letters("A\u{301} β क\u{93c} ① ² \u{345}"), 3);
        let mut english = English {
            status: String::from("present"),
            reference_letters: 4,
        };
        assert_eq!(english_letters(&english, 10).unwrap(), 4);
        assert!(english_letters(&english, 3).is_err());
        english.status = String::from("unknown");
        assert_eq!(english_letters(&english, 10).unwrap(), 10);
        english.status = String::from("absent");
        assert!(english_letters(&english, 10).is_err());
        english.reference_letters = 0;
        assert_eq!(english_letters(&english, 10).unwrap(), 0);
        english.status = String::from("maybe");
        assert!(english_letters(&english, 10).is_err());
    }

    #[test]
    fn sources_require_unique_keys_and_explicit_accepted_rights() {
        let source = r#"{"title":"fixture","url":"https://example.invalid/source","revision":"v1","license":"CC0-1.0","license_evidence_url":"https://example.invalid/license","license_scope":"original test strings","attribution":"test author","admission":"accepted"}"#;
        let mut parsed: Source = serde_json::from_str(source).unwrap();
        validate_source(&parsed).unwrap();
        parsed.attribution.clear();
        assert!(validate_source(&parsed).is_err());
        let duplicate = format!("{{\"a\":{source},\"a\":{source}}}");
        assert!(sources(&mut serde_json::Deserializer::from_str(&duplicate)).is_err());
        let mut excessive = String::from("{");
        for index in 0..33 {
            if index != 0 {
                excessive.push(',');
            }
            use std::fmt::Write as _;
            write!(excessive, "\"s{index}\":{source}").unwrap();
        }
        excessive.push('}');
        assert!(sources(&mut serde_json::Deserializer::from_str(&excessive)).is_err());
    }

    #[test]
    fn manifest_identity_hash_includes_exact_trailing_bytes() {
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!("manifest-hash-test-{nonce}.json"));
        let text = b" {\"key\":1}\n \t";
        use std::io::Write as _;
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
            .unwrap();
        file.write_all(text).unwrap();
        #[derive(Deserialize)]
        struct Value {
            key: u8,
        }
        let (value, digest, bytes): (Value, _, _) = read_json_hashed(&path).unwrap();
        assert_eq!(value.key, 1);
        assert_eq!(digest, format!("{:x}", Sha256::digest(text)));
        assert_eq!(bytes, text.len() as u64);
        file.write_all(b"x").unwrap();
        assert!(read_json_hashed::<Value>(&path).is_err());
        fs::remove_file(path).unwrap();
    }
}
