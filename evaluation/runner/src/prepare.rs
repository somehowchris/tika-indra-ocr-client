//! Reconstruct the preselected corpus from its immutable acquisition locks.
//!
//! Preparation never consults OCR predictions. A generated file is admitted at its
//! frozen path only after both its byte length and SHA-256 match. Failed candidates
//! remain beside that path under their actual hash for diagnosis.

use crate::corpus::{self, Asset, Case, Corpus, CorpusError, Downloads};
use arrayvec::ArrayVec;
use image::{ImageEncoder, ImageReader, codecs::png::PngEncoder};
use quick_xml::{
    Reader, XmlVersion,
    events::{BytesStart, Event},
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    fs::{self, File},
    io::{self, BufWriter, Read, Write},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    time::{Duration, Instant},
};
use thiserror::Error;
use zip::ZipArchive;

static PREPARED_LOCKS: [&str; 10] = [
    "amsmb-prepared.json",
    "bhl-prepared.json",
    "italian-prepared.json",
    "lectaurep-prepared.json",
    "deva-diksita-prepared.json",
    "deva-sankara-prepared.json",
    "deva-siva-prepared.json",
    "bongabdo-prepared.json",
    "chinese-prepared.json",
    "devanagari-body-prepared.json",
];
static MAPPING_LOCKS: [&str; 4] = [
    "bhl-reference-mapped.json",
    "lectaurep-reference-mapped.json",
    "devanagari-reference-mapped.json",
    "chinese-reference-final.json",
];
static MAX_TEXT_BYTES: u64 = 2 * 1024 * 1024;
static MAX_NESTED_ARCHIVE_BYTES: u64 = 160 * 1024 * 1024;
static POPPLER_DEADLINE: Duration = Duration::from_secs(60);

#[derive(Debug, Error)]
pub enum PrepareError {
    #[error(transparent)]
    Corpus(#[from] CorpusError),
    #[error("preparation I/O failed: {0}")]
    Io(#[from] io::Error),
    #[error("bounded preparation allocation failed: {0}")]
    Allocation(#[from] std::collections::TryReserveError),
    #[error("image decode or encode failed: {0}")]
    Image(#[from] image::ImageError),
    #[error("invalid source XML: {0}")]
    Xml(#[from] quick_xml::Error),
    #[error("invalid XML attribute: {0}")]
    Attribute(#[from] quick_xml::events::attributes::AttrError),
    #[error("invalid ZIP archive: {0}")]
    Zip(#[from] zip::result::ZipError),
    #[error("preparation invariant failed: {0}")]
    Invalid(&'static str),
    #[error("{tool} exited unsuccessfully ({status}); no output was admitted")]
    Process { tool: &'static str, status: String },
    #[error("{tool} exceeded its 60-second preparation deadline")]
    Timeout { tool: &'static str },
    #[error(
        "frozen output differs; preserved {candidate}: expected {expected_bytes} bytes / {expected_sha256}, got {actual_bytes} bytes / {actual_sha256}"
    )]
    Mismatch {
        candidate: PathBuf,
        expected_bytes: u64,
        actual_bytes: u64,
        expected_sha256: String,
        actual_sha256: String,
    },
}

#[derive(Debug, Serialize)]
pub struct Failure {
    pub case_id: String,
    pub kind: &'static str,
    pub target: PathBuf,
    pub error: String,
}

#[derive(Debug, Default, Serialize)]
pub struct Preparation {
    pub cases: usize,
    pub verified_files: usize,
    pub generated_files: usize,
    pub reused_files: usize,
    pub failures: ArrayVec<Failure, 256>,
}

#[derive(Debug, Deserialize)]
struct Crop {
    left: u32,
    top: u32,
    width: u32,
    height: u32,
}

#[derive(Debug, Deserialize)]
struct Raster {
    image: Asset,
    source_width: u32,
    source_height: u32,
    width: u32,
    height: u32,
    crop: Option<Crop>,
    conversion: String,
    resampling: String,
    physical_dpi: Option<f64>,
}

#[derive(Debug, Deserialize)]
struct Prepared {
    id: String,
    original_input: Asset,
    original_annotation: Option<Asset>,
    original_reference: Option<Asset>,
    image: Raster,
    reference: Asset,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum PreparedLock {
    Object { prepared: ArrayVec<Prepared, 64> },
    List(ArrayVec<Prepared, 64>),
}

impl PreparedLock {
    fn into_rows(self) -> ArrayVec<Prepared, 64> {
        match self {
            Self::Object { prepared } | Self::List(prepared) => prepared,
        }
    }
}

#[derive(Debug, Deserialize)]
struct Rule {
    from: String,
    to: String,
    expected_occurrences: usize,
}

#[derive(Debug, Deserialize)]
struct Mapping {
    original_reference: Asset,
    reference: Asset,
    rules: ArrayVec<Rule, 16>,
}

#[derive(Debug, Deserialize)]
struct Change {
    codepoint_offset_zero_based: usize,
    from: String,
    to: String,
}

#[derive(Debug, Deserialize)]
struct LegalOverlay {
    id: String,
    original_reference: Asset,
    reference: Asset,
    changes: ArrayVec<Change, 16>,
}

struct Context<'a> {
    root: &'a Path,
    corpus: &'a Corpus,
    prepared: &'a [Prepared],
    mappings: &'a [Mapping],
    legal: &'a [LegalOverlay],
    downloads: &'a Downloads,
}

#[derive(Default)]
struct State {
    report: Preparation,
    verified_archives: ArrayVec<PathBuf, 8>,
}

/// Restore every frozen case and retain a complete report when individual assets fail.
/// The caller must require an empty `failures` array and run corpus admission before OCR.
pub fn prepare(
    corpus: &Corpus,
    root: &Path,
    locks_dir: &Path,
) -> Result<Preparation, PrepareError> {
    if corpus.schema_version != 1 || corpus.status != "frozen" || corpus.cases.is_empty() {
        return Err(PrepareError::Invalid(
            "requires a nonempty frozen schema-1 corpus",
        ));
    }
    let mut prepared: ArrayVec<Prepared, 64> = ArrayVec::new();
    for name in PREPARED_LOCKS {
        let lock: PreparedLock = corpus::read_json(&locks_dir.join(name))?;
        for row in lock.into_rows() {
            prepared
                .try_push(row)
                .map_err(|_| PrepareError::Invalid("too many preparation rows"))?;
        }
    }
    let mut mappings: ArrayVec<Mapping, 32> = ArrayVec::new();
    for name in MAPPING_LOCKS {
        let rows: ArrayVec<Mapping, 32> = corpus::read_json(&locks_dir.join(name))?;
        for row in rows {
            mappings
                .try_push(row)
                .map_err(|_| PrepareError::Invalid("too many reference mappings"))?;
        }
    }
    let legal: ArrayVec<LegalOverlay, 16> =
        corpus::read_json(&locks_dir.join("legal-overlays.json"))?;
    let downloads: Downloads = corpus::read_json(
        &locks_dir
            .parent()
            .ok_or(PrepareError::Invalid("locks directory has no parent"))?
            .join("frozen-downloads.json"),
    )?;
    let context = Context {
        root,
        corpus,
        prepared: &prepared,
        mappings: &mappings,
        legal: &legal,
        downloads: &downloads,
    };
    let mut state = State::default();
    for case in &corpus.cases {
        let image_result = prepare_case_image(&context, &mut state, case);
        record(&mut state.report, case, "image", &case.input, image_result)?;
        let reference_result = ensure_reference(&context, &mut state, &case.reference, 0);
        record(
            &mut state.report,
            case,
            "reference",
            &case.reference,
            reference_result,
        )?;
        state.report.cases += 1;
    }
    Ok(state.report)
}

fn record(
    report: &mut Preparation,
    case: &Case,
    kind: &'static str,
    target: &Asset,
    result: Result<(), PrepareError>,
) -> Result<(), PrepareError> {
    match result {
        Ok(()) => report.verified_files += 1,
        Err(error) => report
            .failures
            .try_push(Failure {
                case_id: String::from(&case.id),
                kind,
                target: PathBuf::from(&target.path),
                error: error.to_string(),
            })
            .map_err(|_| PrepareError::Invalid("too many preparation failures"))?,
    }
    Ok(())
}

fn already_valid(root: &Path, asset: &Asset) -> Result<bool, PrepareError> {
    match corpus::verify_asset(root, asset) {
        Ok(_) => Ok(true),
        Err(CorpusError::Io(error)) if error.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error.into()),
    }
}

fn same_asset(left: &Asset, right: &Asset) -> Result<(), PrepareError> {
    if left.path != right.path || left.bytes != right.bytes || left.sha256 != right.sha256 {
        return Err(PrepareError::Invalid(
            "corpus and preparation asset locks disagree",
        ));
    }
    Ok(())
}

fn prepare_case_image(
    context: &Context<'_>,
    state: &mut State,
    case: &Case,
) -> Result<(), PrepareError> {
    if already_valid(context.root, &case.input)? {
        state.report.reused_files += 1;
        return Ok(());
    }
    if case.source_id == "eurlex" {
        return legal_image(context, state, case);
    }
    if let Some(row) = context
        .prepared
        .iter()
        .find(|row| row.image.image.path == case.input.path)
    {
        same_asset(&row.image.image, &case.input)?;
        if row.id != case.id
            || row.image.conversion != "consuming_L8_grayscale"
            || row.image.resampling != "none"
            || row.image.physical_dpi.is_some()
        {
            return Err(PrepareError::Invalid(
                "unsupported or inconsistent frozen raster transformation",
            ));
        }
        if [row.image.width, row.image.height] != [case.width, case.height] {
            return Err(PrepareError::Invalid(
                "case and preparation dimensions differ",
            ));
        }
        ensure_source(context, state, &row.original_input)?;
        return raster(
            context.root,
            &row.original_input,
            &case.input,
            [row.image.source_width, row.image.source_height],
            [case.width, case.height],
            row.image.crop.as_ref(),
            &mut state.report,
        );
    }
    if case.source_id == "spanish" {
        let source_name = format!("source/spanish/{}.jpg", case.id);
        let source = download_asset(context, Path::new(&source_name))?;
        ensure_source(context, state, source)?;
        return raster(
            context.root,
            source,
            &case.input,
            [case.width, case.height],
            [case.width, case.height],
            None,
            &mut state.report,
        );
    }
    Err(PrepareError::Invalid("no frozen raster preparation record"))
}

fn raster(
    root: &Path,
    source: &Asset,
    target: &Asset,
    source_dimensions: [u32; 2],
    dimensions: [u32; 2],
    crop: Option<&Crop>,
    report: &mut Preparation,
) -> Result<(), PrepareError> {
    let mut reader =
        ImageReader::open(corpus::verify_asset(root, source)?)?.with_guessed_format()?;
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(8192);
    limits.max_image_height = Some(8192);
    limits.max_alloc = Some(256 * 1024 * 1024);
    reader.limits(limits);
    let gray = reader.decode()?.into_luma8();
    if [gray.width(), gray.height()] != source_dimensions {
        return Err(PrepareError::Invalid(
            "decoded source dimensions differ from frozen lock",
        ));
    }
    let (top, height) = if let Some(crop) = crop {
        if crop.left != 0
            || crop.width != gray.width()
            || [crop.width, crop.height] != dimensions
            || crop
                .top
                .checked_add(crop.height)
                .is_none_or(|end| end > gray.height())
        {
            return Err(PrepareError::Invalid(
                "frozen crop must select complete existing rows without resampling",
            ));
        }
        (crop.top, crop.height)
    } else {
        if source_dimensions != dimensions {
            return Err(PrepareError::Invalid("dimensions change without a crop"));
        }
        (0, gray.height())
    };
    // All frozen body crops retain the full source width. Encode their existing
    // contiguous row slice directly; no second raster or crop copy is necessary.
    let start = top as usize * gray.width() as usize;
    let end = start + height as usize * gray.width() as usize;
    let candidate = candidate_path(root, target, "prepare-partial")?;
    let mut output = BufWriter::new(new_file(&candidate)?);
    PngEncoder::new(&mut output).write_image(
        &gray.as_raw()[start..end],
        dimensions[0],
        dimensions[1],
        image::ExtendedColorType::L8,
    )?;
    output.flush()?;
    output.get_ref().sync_all()?;
    drop(output);
    publish(root, target, &candidate, report)
}

fn ensure_reference(
    context: &Context<'_>,
    state: &mut State,
    target: &Asset,
    depth: usize,
) -> Result<(), PrepareError> {
    if depth > 4 {
        return Err(PrepareError::Invalid("reference preparation cycle"));
    }
    if already_valid(context.root, target)? {
        state.report.reused_files += 1;
        return Ok(());
    }
    if target.path.starts_with("source") {
        return ensure_source(context, state, target);
    }
    if let Some(mapping) = context
        .mappings
        .iter()
        .find(|row| row.reference.path == target.path)
    {
        same_asset(&mapping.reference, target)?;
        ensure_reference(context, state, &mapping.original_reference, depth + 1)?;
        let mut text = read_text(context.root, &mapping.original_reference)?;
        apply_rules(&mut text, &mapping.rules)?;
        return write_text(context.root, target, &text, &mut state.report);
    }
    if let Some(overlay) = context
        .legal
        .iter()
        .find(|row| row.reference.path == target.path)
    {
        same_asset(&overlay.reference, target)?;
        ensure_reference(context, state, &overlay.original_reference, depth + 1)?;
        let text = read_text(context.root, &overlay.original_reference)?;
        let corrected = apply_codepoint_changes(&text, &overlay.changes)?;
        return write_text(context.root, target, &corrected, &mut state.report);
    }
    if let Some(overlay) = context
        .legal
        .iter()
        .find(|row| row.original_reference.path == target.path)
    {
        same_asset(&overlay.original_reference, target)?;
        let case = context
            .corpus
            .cases
            .iter()
            .find(|case| case.id == overlay.id)
            .ok_or(PrepareError::Invalid("legal overlay has no frozen case"))?;
        return legal_text(context, state, case, target);
    }
    if let Some(row) = context
        .prepared
        .iter()
        .find(|row| row.reference.path == target.path)
    {
        same_asset(&row.reference, target)?;
        if let Some(source) = &row.original_reference {
            ensure_reference(context, state, source, depth + 1)?;
            let text = read_text(context.root, source)?;
            return write_text(context.root, target, &text, &mut state.report);
        }
        let annotation = row
            .original_annotation
            .as_ref()
            .ok_or(PrepareError::Invalid(
                "reference has neither annotation nor source text",
            ))?;
        ensure_source(context, state, annotation)?;
        let source = read_text(context.root, annotation)?;
        let text = xml_reference(&source, row.id.starts_with("bhl-"))?;
        return write_text(context.root, target, &text, &mut state.report);
    }
    if let Some(case) = context
        .corpus
        .cases
        .iter()
        .find(|case| case.reference.path == target.path && case.source_id == "spanish")
    {
        let annotation = case
            .annotation
            .as_ref()
            .ok_or(PrepareError::Invalid("Spanish annotation missing"))?;
        ensure_source(context, state, annotation)?;
        let source = read_text(context.root, annotation)?;
        let text = xml_reference(&source, false)?;
        return write_text(context.root, target, &text, &mut state.report);
    }
    Err(PrepareError::Invalid(
        "no frozen reference preparation record",
    ))
}

fn read_text(root: &Path, source: &Asset) -> Result<String, PrepareError> {
    if source.bytes > MAX_TEXT_BYTES {
        return Err(PrepareError::Invalid("source text exceeds bound"));
    }
    Ok(fs::read_to_string(corpus::verify_asset(root, source)?)?)
}

fn write_text(
    root: &Path,
    target: &Asset,
    text: &str,
    report: &mut Preparation,
) -> Result<(), PrepareError> {
    if text.len() as u64 > MAX_TEXT_BYTES {
        return Err(PrepareError::Invalid("prepared text exceeds bound"));
    }
    let candidate = candidate_path(root, target, "prepare-partial")?;
    let mut output = new_file(&candidate)?;
    output.write_all(text.as_bytes())?;
    output.sync_all()?;
    drop(output);
    publish(root, target, &candidate, report)
}

fn apply_rules(text: &mut String, rules: &[Rule]) -> Result<(), PrepareError> {
    for rule in rules {
        if rule.from.is_empty() || text.matches(&rule.from).count() != rule.expected_occurrences {
            return Err(PrepareError::Invalid(
                "reference replacement occurrence count differs",
            ));
        }
        let retained = text
            .len()
            .checked_sub(
                rule.from
                    .len()
                    .checked_mul(rule.expected_occurrences)
                    .ok_or(PrepareError::Invalid("replacement size overflow"))?,
            )
            .ok_or(PrepareError::Invalid("replacement size underflow"))?;
        let required = retained
            .checked_add(
                rule.to
                    .len()
                    .checked_mul(rule.expected_occurrences)
                    .ok_or(PrepareError::Invalid("replacement size overflow"))?,
            )
            .ok_or(PrepareError::Invalid("replacement size overflow"))?;
        if required as u64 > MAX_TEXT_BYTES {
            return Err(PrepareError::Invalid(
                "reference replacement would exceed text bound",
            ));
        }
        text.try_reserve_exact(required.saturating_sub(text.len()))?;
        // The mapping is an audited textual transformation, not normalization.
        // Reuse the same buffer; search resumes after the inserted replacement.
        let mut offset = 0;
        while let Some(relative) = text[offset..].find(&rule.from) {
            let start = offset + relative;
            text.replace_range(start..start + rule.from.len(), &rule.to);
            offset = start + rule.to.len();
        }
    }
    Ok(())
}

fn apply_codepoint_changes(text: &str, changes: &[Change]) -> Result<String, PrepareError> {
    let mut output = String::with_capacity(text.len());
    let mut applied = 0;
    for (position, character) in text.chars().enumerate() {
        if let Some(change) = changes.get(applied) {
            if change.codepoint_offset_zero_based < position {
                return Err(PrepareError::Invalid(
                    "legal changes are unordered or duplicated",
                ));
            }
            if change.codepoint_offset_zero_based == position {
                let mut from = change.from.chars();
                if from.next() != Some(character) || from.next().is_some() {
                    return Err(PrepareError::Invalid(
                        "legal overlay source codepoint differs",
                    ));
                }
                output.push_str(&change.to);
                applied += 1;
                continue;
            }
        }
        output.push(character);
    }
    if applied != changes.len() {
        return Err(PrepareError::Invalid(
            "legal overlay offset exceeds reference",
        ));
    }
    Ok(output)
}

fn download_asset<'a>(context: &'a Context<'_>, path: &Path) -> Result<&'a Asset, PrepareError> {
    context
        .downloads
        .assets
        .iter()
        .find(|entry| entry.asset.path == path)
        .map(|entry| &entry.asset)
        .ok_or(PrepareError::Invalid(
            "source archive is absent from frozen download lock",
        ))
}

fn checked_archive(
    context: &Context<'_>,
    state: &mut State,
    name: &str,
) -> Result<PathBuf, PrepareError> {
    let asset = download_asset(context, Path::new(name))?;
    if !state.verified_archives.contains(&asset.path) {
        corpus::verify_asset(context.root, asset)?;
        state
            .verified_archives
            .try_push(PathBuf::from(&asset.path))
            .map_err(|_| PrepareError::Invalid("too many source archives"))?;
    }
    Ok(corpus::safe_path(context.root, &asset.path)?)
}

fn ensure_source(
    context: &Context<'_>,
    state: &mut State,
    asset: &Asset,
) -> Result<(), PrepareError> {
    if already_valid(context.root, asset)? {
        return Ok(());
    }
    let path = asset
        .path
        .to_str()
        .ok_or(PrepareError::Invalid("non-UTF-8 source path"))?;
    if let Some(file) = path.strip_prefix("source/devanagari/") {
        let (stem, extension) = file
            .rsplit_once('.')
            .ok_or(PrepareError::Invalid("source extension missing"))?;
        let (book, page) = stem
            .rsplit_once('-')
            .ok_or(PrepareError::Invalid("source page missing"))?;
        let archive = checked_archive(context, state, &format!("archives/devanagari-{book}.zip"))?;
        let entry = if extension == "xml" {
            format!("{book}/{book}/alto/{page}.xml")
        } else if extension == "jpg" {
            format!("{book}/{book}/{page}.jpg")
        } else {
            return Err(PrepareError::Invalid("unexpected Devanagari source type"));
        };
        return extract_locked(context.root, &archive, &entry, asset, &mut state.report);
    }
    if let Some(file) = path.strip_prefix("source/chinese/jingbao-") {
        let (year, crop) = file
            .split_once('-')
            .ok_or(PrepareError::Invalid("Chinese crop identifier missing"))?;
        let archive = checked_archive(context, state, "archives/chinese-jingbao-crops.zip")?;
        return extract_locked(
            context.root,
            &archive,
            &format!("ann_crops_{year}/{crop}"),
            asset,
            &mut state.report,
        );
    }
    if let Some(file) = path.strip_prefix("source/bongabdo/") {
        let outer = checked_archive(context, state, "archives/bongabdo.zip")?;
        let nested =
            corpus::safe_path(context.root, Path::new("source/bongabdo/Bongabdo1429.zip"))?;
        if !state.verified_archives.contains(&nested) {
            let mut archive = ZipArchive::new(File::open(outer)?)?;
            let mut entry = archive.by_name("Bongabdo1429.zip")?;
            if entry.size() > MAX_NESTED_ARCHIVE_BYTES {
                return Err(PrepareError::Invalid("nested archive exceeds bound"));
            }
            let partial = nested.with_extension("zip.prepare-partial");
            let mut output = new_file(&partial)?;
            let actual = copy_bounded(&mut entry, &mut output, MAX_NESTED_ARCHIVE_BYTES)?;
            if actual != entry.size() {
                return Err(PrepareError::Invalid("nested archive is truncated"));
            }
            output.sync_all()?;
            drop(output);
            promote_identical(&partial, &nested)?;
            state
                .verified_archives
                .try_push(PathBuf::from(&nested))
                .map_err(|_| PrepareError::Invalid("too many archives"))?;
        }
        let directory = if file.ends_with(".jpg") {
            "Images"
        } else if file.ends_with(".txt") {
            "Annotations"
        } else {
            return Err(PrepareError::Invalid("unexpected Bongabdo source type"));
        };
        return extract_locked(
            context.root,
            &nested,
            &format!("{directory}/{file}"),
            asset,
            &mut state.report,
        );
    }
    // Direct downloads are the fetch command's responsibility. Never replace an
    // unavailable source with another scan or a different publisher response.
    corpus::verify_asset(context.root, asset)?;
    Ok(())
}

fn extract_locked(
    root: &Path,
    archive: &Path,
    entry_name: &str,
    target: &Asset,
    report: &mut Preparation,
) -> Result<(), PrepareError> {
    let mut archive = ZipArchive::new(File::open(archive)?)?;
    let mut entry = archive.by_name(entry_name)?;
    if entry.is_dir() || entry.size() != target.bytes {
        return Err(PrepareError::Invalid(
            "archive entry size differs from source lock",
        ));
    }
    let candidate = candidate_path(root, target, "prepare-partial")?;
    let mut output = new_file(&candidate)?;
    let copied = copy_bounded(&mut entry, &mut output, target.bytes)?;
    if copied != target.bytes {
        return Err(PrepareError::Invalid("archive entry is truncated"));
    }
    output.sync_all()?;
    drop(output);
    publish(root, target, &candidate, report)
}

fn copy_bounded(
    input: &mut impl Read,
    output: &mut impl Write,
    limit: u64,
) -> Result<u64, PrepareError> {
    let mut buffer = [0_u8; 65_536];
    let mut count = 0_u64;
    loop {
        let read = input.read(&mut buffer)?;
        if read == 0 {
            return Ok(count);
        }
        count += read as u64;
        if count > limit {
            return Err(PrepareError::Invalid("decompressed input exceeds bound"));
        }
        output.write_all(&buffer[..read])?;
    }
}

fn legal_page(case: &Case) -> Result<&str, PrepareError> {
    let (_, page) = case
        .id
        .rsplit_once("-p")
        .ok_or(PrepareError::Invalid("legal page suffix missing"))?;
    if page.len() != 4 || !page.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(PrepareError::Invalid("invalid legal page number"));
    }
    Ok(page)
}

fn legal_image(context: &Context<'_>, state: &mut State, case: &Case) -> Result<(), PrepareError> {
    let annotation = case
        .annotation
        .as_ref()
        .ok_or(PrepareError::Invalid("legal source PDF missing"))?;
    let source = corpus::verify_asset(context.root, annotation)?;
    let page = legal_page(case)?;
    let candidate = candidate_path(context.root, &case.input, "prepare-partial.png")?;
    let output = new_file(&candidate)?;
    run_tool(
        Command::new("pdftoppm")
            .args([
                "-f",
                page,
                "-l",
                page,
                "-r",
                "300",
                "-gray",
                "-png",
                "-singlefile",
            ])
            .arg(source),
        "pdftoppm",
        output,
    )?;
    File::open(&candidate)?.sync_all()?;
    let dimensions = image::image_dimensions(&candidate)?;
    if dimensions != (case.width, case.height) {
        return Err(PrepareError::Invalid("legal raster dimensions differ"));
    }
    publish(context.root, &case.input, &candidate, &mut state.report)
}

fn legal_text(
    context: &Context<'_>,
    state: &mut State,
    case: &Case,
    target: &Asset,
) -> Result<(), PrepareError> {
    let annotation = case
        .annotation
        .as_ref()
        .ok_or(PrepareError::Invalid("legal source PDF missing"))?;
    let source = corpus::verify_asset(context.root, annotation)?;
    let page = legal_page(case)?;
    let candidate = candidate_path(context.root, target, "prepare-partial")?;
    let output = new_file(&candidate)?;
    run_tool(
        Command::new("pdftotext")
            .args(["-f", page, "-l", page, "-layout", "-enc", "UTF-8"])
            .arg(source)
            .arg("-"),
        "pdftotext",
        output,
    )?;
    File::open(&candidate)?.sync_all()?;
    publish(context.root, target, &candidate, &mut state.report)
}

fn run_tool(command: &mut Command, tool: &'static str, output: File) -> Result<(), PrepareError> {
    let mut child = command
        .stdin(Stdio::null())
        .stdout(output)
        .stderr(Stdio::inherit())
        .spawn()?;
    let started = Instant::now();
    let result = loop {
        match child.try_wait() {
            Ok(Some(status)) => break Ok(status),
            Ok(None) if started.elapsed() >= POPPLER_DEADLINE => {
                break Err(PrepareError::Timeout { tool });
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(10)),
            Err(error) => break Err(PrepareError::Io(error)),
        }
    };
    let status = match result {
        Ok(status) => status,
        Err(error) => {
            child.kill()?;
            child.wait()?;
            return Err(error);
        }
    };
    if !status.success() {
        return Err(PrepareError::Process {
            tool,
            status: status.to_string(),
        });
    }
    Ok(())
}

fn candidate_path(root: &Path, target: &Asset, suffix: &str) -> Result<PathBuf, PrepareError> {
    let target_path = corpus::safe_path(root, &target.path)?;
    let file_name = target_path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or(PrepareError::Invalid("invalid output name"))?;
    Ok(target_path.with_file_name(format!("{file_name}.{suffix}")))
}

fn new_file(path: &Path) -> Result<File, PrepareError> {
    fs::create_dir_all(
        path.parent()
            .ok_or(PrepareError::Invalid("output has no parent"))?,
    )?;
    Ok(corpus::create_partial(path)?)
}

fn publish(
    root: &Path,
    target: &Asset,
    candidate: &Path,
    report: &mut Preparation,
) -> Result<(), PrepareError> {
    let (bytes, sha256) = file_digest(candidate)?;
    if bytes != target.bytes || sha256 != target.sha256 {
        let preserved = candidate_path(root, target, &format!("{sha256}.reconstructed"))?;
        promote_identical(candidate, &preserved)?;
        return Err(PrepareError::Mismatch {
            candidate: preserved,
            expected_bytes: target.bytes,
            actual_bytes: bytes,
            expected_sha256: String::from(&target.sha256),
            actual_sha256: sha256,
        });
    }
    let destination = corpus::safe_path(root, &target.path)?;
    if promote_identical(candidate, &destination)? {
        report.generated_files += 1;
    } else {
        report.reused_files += 1;
    }
    Ok(())
}

fn file_digest(path: &Path) -> Result<(u64, String), PrepareError> {
    let metadata = fs::symlink_metadata(path)?;
    if !metadata.is_file() {
        return Err(PrepareError::Invalid("hash target must be a regular file"));
    }
    let mut source = File::open(path)?;
    let mut digest = Sha256::new();
    let mut buffer = [0_u8; 65_536];
    let mut bytes = 0_u64;
    loop {
        let read = source.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        bytes += read as u64;
        digest.update(&buffer[..read]);
    }
    Ok((bytes, format!("{:x}", digest.finalize())))
}

/// Atomic same-directory admission never replaces a concurrently created file.
/// Reuse is allowed only when both existing and candidate contents are identical.
fn promote_identical(candidate: &Path, destination: &Path) -> Result<bool, PrepareError> {
    let created = match fs::hard_link(candidate, destination) {
        Ok(()) => true,
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
            if file_digest(candidate)? != file_digest(destination)? {
                return Err(PrepareError::Invalid(
                    "existing output differs; candidate preserved without overwriting",
                ));
            }
            false
        }
        Err(error) => return Err(error.into()),
    };
    fs::remove_file(candidate)?;
    File::open(
        destination
            .parent()
            .ok_or(PrepareError::Invalid("output has no parent"))?,
    )?
    .sync_all()?;
    Ok(created)
}

#[derive(Default)]
struct Region {
    id: String,
    kind: String,
    text: String,
    fallback: String,
    lines: usize,
}

#[derive(Debug)]
struct RegionOrder {
    index: usize,
    id: String,
}

fn attribute(element: &BytesStart<'_>, name: &str) -> Result<String, PrepareError> {
    for attribute in element.attributes() {
        let attribute = attribute?;
        if attribute.key.local_name().as_ref() == name {
            return Ok(String::from(
                attribute
                    .normalized_value(XmlVersion::Implicit1_0)?
                    .as_ref(),
            ));
        }
    }
    Ok(String::new())
}

/// PAGE retains empty regions; ALTO joins String contents within each TextLine.
/// BHL's explicit body order is extended only by its audited running matter.
fn xml_reference(source: &str, bhl_order: bool) -> Result<String, PrepareError> {
    if source.len() as u64 > MAX_TEXT_BYTES {
        return Err(PrepareError::Invalid("XML exceeds bound"));
    }
    let mut reader = Reader::from_str(source);
    reader.config_mut().expand_empty_elements = true;
    let mut regions: ArrayVec<Region, 512> = ArrayVec::new();
    let mut order: ArrayVec<RegionOrder, 512> = ArrayVec::new();
    let mut current: Option<Region> = None;
    let mut in_line = false;
    let mut in_unicode = false;
    let mut line_words = 0;
    loop {
        match reader.read_event()? {
            Event::Start(element) | Event::Empty(element) => match element.local_name().as_ref() {
                "TextRegion" | "TextBlock" => {
                    if current.is_some() {
                        return Err(PrepareError::Invalid("nested text regions are unsupported"));
                    }
                    let mut id = attribute(&element, "id")?;
                    if id.is_empty() {
                        id = attribute(&element, "ID")?;
                    }
                    current = Some(Region {
                        id,
                        kind: attribute(&element, "type")?,
                        ..Region::default()
                    });
                }
                "TextLine" => {
                    let region = current
                        .as_mut()
                        .ok_or(PrepareError::Invalid("text line has no region"))?;
                    if in_line {
                        return Err(PrepareError::Invalid("nested text lines"));
                    }
                    if region.lines != 0 {
                        region.text.push('\n');
                    }
                    region.lines += 1;
                    line_words = 0;
                    in_line = true;
                }
                "Unicode" if current.is_some() => {
                    if in_unicode {
                        return Err(PrepareError::Invalid("nested Unicode element"));
                    }
                    in_unicode = true;
                }
                "String" if in_line => {
                    let region = current
                        .as_mut()
                        .ok_or(PrepareError::Invalid("word has no region"))?;
                    if line_words != 0 {
                        region.text.push(' ');
                    }
                    region.text.push_str(&attribute(&element, "CONTENT")?);
                    line_words += 1;
                }
                "RegionRefIndexed" if bhl_order => {
                    let index = attribute(&element, "index")?
                        .parse()
                        .map_err(|_| PrepareError::Invalid("invalid region order index"))?;
                    order
                        .try_push(RegionOrder {
                            index,
                            id: attribute(&element, "regionRef")?,
                        })
                        .map_err(|_| PrepareError::Invalid("too many ordered regions"))?;
                }
                _ => (),
            },
            Event::End(element) => match element.local_name().as_ref() {
                "Unicode" => in_unicode = false,
                "TextLine" => in_line = false,
                "TextRegion" | "TextBlock" => {
                    let mut region = current
                        .take()
                        .ok_or(PrepareError::Invalid("region end has no start"))?;
                    if region.lines == 0 {
                        region.text = std::mem::take(&mut region.fallback);
                    }
                    regions
                        .try_push(region)
                        .map_err(|_| PrepareError::Invalid("too many text regions"))?;
                }
                _ => (),
            },
            Event::Text(text) if in_unicode => {
                let region = current
                    .as_mut()
                    .ok_or(PrepareError::Invalid("text has no region"))?;
                let target = if in_line {
                    &mut region.text
                } else {
                    &mut region.fallback
                };
                target.push_str(&text.xml10_content());
            }
            Event::CData(text) if in_unicode => {
                let region = current
                    .as_mut()
                    .ok_or(PrepareError::Invalid("CDATA has no region"))?;
                let target = if in_line {
                    &mut region.text
                } else {
                    &mut region.fallback
                };
                target.push_str(&text.xml10_content());
            }
            Event::GeneralRef(reference) if in_unicode => {
                let character = if let Some(character) = reference.resolve_char_ref()? {
                    character
                } else {
                    match reference.as_ref() {
                        "amp" => '&',
                        "lt" => '<',
                        "gt" => '>',
                        "quot" => '"',
                        "apos" => '\'',
                        _ => return Err(PrepareError::Invalid("unsupported XML entity")),
                    }
                };
                let region = current
                    .as_mut()
                    .ok_or(PrepareError::Invalid("entity has no region"))?;
                let target = if in_line {
                    &mut region.text
                } else {
                    &mut region.fallback
                };
                target.push(character);
            }
            Event::DocType(_) => {
                return Err(PrepareError::Invalid(
                    "DTD and external entities are unsupported",
                ));
            }
            Event::Eof => break,
            _ => (),
        }
    }
    if current.is_some() || regions.is_empty() {
        return Err(PrepareError::Invalid("incomplete or empty annotation"));
    }
    let mut indices: ArrayVec<usize, 512> = ArrayVec::new();
    if bhl_order {
        order.sort_unstable_by_key(|entry| entry.index);
        for pair in order.windows(2) {
            if pair[0].index == pair[1].index {
                return Err(PrepareError::Invalid("duplicate region order index"));
            }
        }
        for (index, region) in regions.iter().enumerate() {
            if !order.iter().any(|entry| entry.id == region.id)
                && matches!(region.kind.as_str(), "header" | "page-number")
            {
                indices
                    .try_push(index)
                    .map_err(|_| PrepareError::Invalid("too many region indices"))?;
            }
        }
        for entry in &order {
            let index = regions
                .iter()
                .position(|region| region.id == entry.id)
                .ok_or(PrepareError::Invalid(
                    "reading order references absent region",
                ))?;
            if indices.contains(&index) {
                return Err(PrepareError::Invalid("reading order repeats region"));
            }
            indices
                .try_push(index)
                .map_err(|_| PrepareError::Invalid("too many region indices"))?;
        }
        for (index, region) in regions.iter().enumerate() {
            if !indices.contains(&index) {
                if region.kind != "footnote" {
                    return Err(PrepareError::Invalid(
                        "unclassified BHL region outside reading order",
                    ));
                }
                indices
                    .try_push(index)
                    .map_err(|_| PrepareError::Invalid("too many region indices"))?;
            }
        }
    } else {
        for index in 0..regions.len() {
            indices.push(index);
        }
    }
    let text_bytes: usize = regions.iter().map(|region| region.text.len()).sum();
    let mut output = String::with_capacity(text_bytes + regions.len().saturating_sub(1));
    for (position, index) in indices.into_iter().enumerate() {
        if position != 0 {
            output.push('\n');
        }
        output.push_str(&regions[index].text);
    }
    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn page_keeps_empty_regions_and_decodes_one_entity_layer() {
        let xml = "<PcGts><Page><TextRegion id='a'><TextLine><TextEquiv><Unicode>A&amp;amp;B&#xE9;</Unicode></TextEquiv></TextLine></TextRegion><TextRegion id='b'><TextEquiv><Unicode/></TextEquiv></TextRegion><TextRegion id='c'><TextLine><TextEquiv><Unicode>C</Unicode></TextEquiv></TextLine></TextRegion></Page></PcGts>";
        assert_eq!(xml_reference(xml, false).expect("PAGE"), "A&amp;Bé\n\nC");
    }

    #[test]
    fn alto_joins_words_lines_and_regions_without_terminal_newline() {
        let xml = "<alto><TextBlock ID='a'><TextLine><String CONTENT='एक'/><String CONTENT='दो'/></TextLine><TextLine><String CONTENT='तीन'/></TextLine></TextBlock><TextBlock ID='b'><TextLine><String CONTENT='चार'/></TextLine></TextBlock></alto>";
        assert_eq!(xml_reference(xml, false).expect("ALTO"), "एक दो\nतीन\nचार");
    }

    #[test]
    fn bhl_extends_explicit_body_order_with_running_matter() {
        let xml = "<PcGts><ReadingOrder><RegionRefIndexed regionRef='body' index='0'/></ReadingOrder><TextRegion id='foot' type='footnote'><Unicode>note</Unicode></TextRegion><TextRegion id='body' type='paragraph'><Unicode>body</Unicode></TextRegion><TextRegion id='head' type='header'><Unicode>header</Unicode></TextRegion></PcGts>";
        assert_eq!(
            xml_reference(xml, true).expect("BHL order"),
            "header\nbody\nnote"
        );
    }

    #[test]
    fn mapping_checks_counts_and_does_not_reprocess_inserted_text() {
        let mut text = String::from("&amp; and &amp;");
        let rule = Rule {
            from: String::from("&amp;"),
            to: String::from("&amp;&"),
            expected_occurrences: 2,
        };
        apply_rules(&mut text, &[rule]).expect("mapping");
        assert_eq!(text, "&amp;& and &amp;&");
        let bad = Rule {
            from: String::from("missing"),
            to: String::new(),
            expected_occurrences: 1,
        };
        assert!(apply_rules(&mut text, &[bad]).is_err());
    }

    #[test]
    fn oversized_replacement_is_rejected_before_mutation_or_growth() {
        let mut text = String::from("aa");
        let capacity = text.capacity();
        let rule = Rule {
            from: String::from("a"),
            to: "b".repeat(MAX_TEXT_BYTES as usize / 2 + 1),
            expected_occurrences: 2,
        };
        assert!(apply_rules(&mut text, &[rule]).is_err());
        assert_eq!(text, "aa");
        assert_eq!(text.capacity(), capacity);
    }

    #[test]
    fn promotion_never_overwrites_different_outputs_and_reuses_identical_bytes() {
        let root =
            std::env::temp_dir().join(format!("preparation-promotion-test-{}", std::process::id()));
        fs::create_dir(&root).expect("isolated directory");
        let candidate = root.join("candidate");
        let destination = root.join("destination");
        new_file(&candidate).unwrap().write_all(b"new").unwrap();
        new_file(&destination).unwrap().write_all(b"old").unwrap();
        assert!(promote_identical(&candidate, &destination).is_err());
        assert_eq!(fs::read_to_string(&destination).unwrap(), "old");
        assert_eq!(fs::read_to_string(&candidate).unwrap(), "new");
        fs::remove_file(&destination).unwrap();
        assert!(promote_identical(&candidate, &destination).unwrap());
        assert!(!candidate.exists());
        new_file(&candidate).unwrap().write_all(b"new").unwrap();
        assert!(!promote_identical(&candidate, &destination).unwrap());
        assert!(!candidate.exists());
        assert_eq!(fs::read_to_string(&destination).unwrap(), "new");
        #[cfg(unix)]
        {
            let dangling = root.join("dangling");
            std::os::unix::fs::symlink(root.join("absent"), &dangling).unwrap();
            new_file(&candidate).unwrap().write_all(b"new").unwrap();
            assert!(promote_identical(&candidate, &dangling).is_err());
            assert!(
                fs::symlink_metadata(&dangling)
                    .unwrap()
                    .file_type()
                    .is_symlink()
            );
            assert!(candidate.exists());
            assert!(new_file(&dangling).is_err());
            assert!(!root.join("absent").exists());
        }
        fs::remove_dir_all(root).expect("remove only isolated test outputs");
    }

    #[test]
    fn legal_offsets_count_codepoints_and_validate_original_glyph() {
        let changes = [Change {
            codepoint_offset_zero_based: 2,
            from: String::from("\u{ad}"),
            to: String::from("-"),
        }];
        assert_eq!(
            apply_codepoint_changes("éα\u{ad}β", &changes).expect("overlay"),
            "éα-β"
        );
        assert!(apply_codepoint_changes("éα β", &changes).is_err());
    }

    #[test]
    fn annotation_rejects_external_entities_and_ambiguous_order() {
        assert!(xml_reference("<!DOCTYPE x SYSTEM 'file:///etc/passwd'><x/>", false).is_err());
        let xml = "<PcGts><TextRegion id='body' type='paragraph'><Unicode>body</Unicode></TextRegion></PcGts>";
        assert!(xml_reference(xml, true).is_err());
    }
}
