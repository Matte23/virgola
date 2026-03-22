use std::borrow::Cow;
use std::fmt;
use std::fs;
use std::io::Cursor;
use std::path::Path;

// ── Error type ────────────────────────────────────────────────────────────────

#[derive(Debug)]
pub enum CsvError {
    Io(std::io::Error),
    Parse(csv::Error),
}

impl fmt::Display for CsvError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CsvError::Io(e) => write!(f, "I/O error: {e}"),
            CsvError::Parse(e) => write!(f, "CSV error: {e}"),
        }
    }
}

impl std::error::Error for CsvError {}

impl From<csv::Error> for CsvError {
    fn from(e: csv::Error) -> Self {
        CsvError::Parse(e)
    }
}

impl From<std::io::Error> for CsvError {
    fn from(e: std::io::Error) -> Self {
        CsvError::Io(e)
    }
}

// ── Read result ───────────────────────────────────────────────────────────────

/// Returned by `read_csv`.  Carries the parsed data plus diagnostic flags
/// so the UI can warn the user without refusing to load the file.
pub struct CsvReadResult {
    pub headers: Vec<String>,
    pub rows: Vec<Vec<String>>,
    /// `true` if at least one data row had a different number of fields than
    /// the header row.
    pub had_jagged_rows: bool,
    /// The encoding that was used to decode the file (auto-detected or
    /// explicitly requested via `encoding_hint`).
    pub encoding: &'static encoding_rs::Encoding,
    /// `true` if the file began with a byte-order mark for this encoding.
    pub encoding_bom: bool,
}

// ── Encoding ──────────────────────────────────────────────────────────────────

/// Auto-detect the encoding of `raw` bytes, decode to UTF-8, and report
/// the detected encoding and whether a BOM was present.
///
/// Fast path: valid UTF-8 (optionally with a UTF-8 BOM that is stripped)
/// returns a borrowed slice with no allocation.
///
/// Slow path: `chardetng` guesses the encoding and `encoding_rs` decodes it.
/// Unmappable bytes are replaced with U+FFFD.
fn decode_bytes(raw: &[u8]) -> (Cow<'_, str>, &'static encoding_rs::Encoding, bool) {
    // UTF-8 BOM (\xEF\xBB\xBF) — strip before validity check.
    if let Some(rest) = raw.strip_prefix(b"\xEF\xBB\xBF")
        && let Ok(s) = std::str::from_utf8(rest)
    {
        return (Cow::Borrowed(s), encoding_rs::UTF_8, true);
    }
    // Has a UTF-8 BOM but invalid UTF-8 after it — fall through to detection.

    // Fast path: valid UTF-8 without BOM.
    if let Ok(s) = std::str::from_utf8(raw) {
        return (Cow::Borrowed(s), encoding_rs::UTF_8, false);
    }

    // Non-UTF-8: detect encoding and decode.
    let mut det = chardetng::EncodingDetector::new();
    det.feed(raw, true);
    let encoding = det.guess(None, true);

    // `encoding_rs::Encoding::decode` strips UTF-16 LE/BE BOMs automatically.
    let had_bom = raw.starts_with(b"\xFF\xFE") || raw.starts_with(b"\xFE\xFF");
    let (decoded, _, _) = encoding.decode(raw);

    // Strip any residual U+FEFF (defensive — encoding_rs should have removed it).
    let decoded = strip_bom_char(decoded);
    (decoded, encoding, had_bom)
}

/// Decode `raw` using an *explicitly specified* encoding, stripping the BOM
/// when `bom` is true.  Used when the user overrides the auto-detected encoding.
fn decode_as<'a>(raw: &'a [u8], enc: &'static encoding_rs::Encoding, bom: bool) -> Cow<'a, str> {
    if enc == encoding_rs::UTF_8 {
        let payload = if bom {
            raw.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(raw)
        } else {
            raw
        };
        if let Ok(s) = std::str::from_utf8(payload) {
            return Cow::Borrowed(s);
        }
        // User said UTF-8 but the bytes aren't — decode lossily rather than fail.
        return Cow::Owned(String::from_utf8_lossy(payload).into_owned());
    }

    // Non-UTF-8: encoding_rs strips UTF-16 BOMs automatically.
    let (decoded, _, _) = enc.decode(raw);
    strip_bom_char(decoded)
}

/// Remove a leading U+FEFF from a `Cow<str>` in place.
fn strip_bom_char(s: Cow<'_, str>) -> Cow<'_, str> {
    const BOM: char = '\u{FEFF}';
    match s {
        Cow::Borrowed(s) => Cow::Borrowed(s.strip_prefix(BOM).unwrap_or(s)),
        Cow::Owned(s) => Cow::Owned(if s.starts_with(BOM) {
            s[BOM.len_utf8()..].to_owned()
        } else {
            s
        }),
    }
}

// ── Public API ────────────────────────────────────────────────────────────────

/// Read a CSV file.
///
/// Pass `encoding_hint = None` to auto-detect the encoding (the usual case
/// when first opening a file).  Pass `Some((enc, bom))` to force a specific
/// encoding — used when the user changes the encoding dropdown.
pub fn read_csv(
    path: &Path,
    sep: u8,
    has_header: bool,
    encoding_hint: Option<(&'static encoding_rs::Encoding, bool)>,
) -> Result<CsvReadResult, CsvError> {
    let raw = fs::read(path)?;

    let (text, encoding, encoding_bom): (Cow<str>, _, _) = match encoding_hint {
        Some((enc, bom)) => (decode_as(&raw, enc, bom), enc, bom),
        None => decode_bytes(&raw),
    };

    let mut rdr = csv::ReaderBuilder::new()
        .delimiter(sep)
        .has_headers(has_header)
        // flexible(true) so we don't hard-reject jagged files; we report the
        // condition via CsvReadResult::had_jagged_rows instead.
        .flexible(true)
        .from_reader(Cursor::new(text.as_bytes()));

    let headers: Vec<String>;
    let mut rows: Vec<Vec<String>> = Vec::new();

    if has_header {
        headers = rdr.headers()?.iter().map(|s| s.to_string()).collect();
        for result in rdr.records() {
            let record = result?;
            rows.push(record.iter().map(|s| s.to_string()).collect());
        }
    } else {
        // No header row in the file.
        // Read all records first to determine column count from the first row.
        for result in rdr.records() {
            let record = result?;
            rows.push(record.iter().map(|s| s.to_string()).collect());
        }
        let ncols = rows.first().map(|r| r.len()).unwrap_or(0);
        headers = (1..=ncols).map(|i| format!("Column {}", i)).collect();
    }

    let ncols = headers.len();
    let had_jagged_rows = rows.iter().any(|r| r.len() != ncols);

    Ok(CsvReadResult {
        headers,
        rows,
        had_jagged_rows,
        encoding,
        encoding_bom,
    })
}

/// Sniff the separator used in a CSV file by sampling the first few lines.
///
/// Scores each candidate byte (`,` `;` `\t` `|`) by how many times it appears
/// across the sampled lines and whether its per-line count is consistent.
/// Returns the best-scoring candidate, falling back to `,` on any error or
/// when no candidate scores above zero.
pub fn detect_separator(path: &Path) -> u8 {
    const CANDIDATES: &[u8] = b",;\t|";
    const MAX_LINES: usize = 20;
    const SAMPLE_BYTES: usize = 8192;

    let content = match fs::read(path) {
        Ok(c) => c,
        Err(_) => return b',',
    };
    let sample = &content[..content.len().min(SAMPLE_BYTES)];
    let text = String::from_utf8_lossy(sample);

    let lines: Vec<&str> = text
        .lines()
        .filter(|l| !l.trim().is_empty())
        .take(MAX_LINES)
        .collect();

    if lines.is_empty() {
        return b',';
    }

    let mut best_sep = b',';
    let mut best_score: usize = 0;

    for &sep in CANDIDATES {
        let counts: Vec<usize> = lines
            .iter()
            .map(|line| line.bytes().filter(|&b| b == sep).count())
            .collect();

        let total: usize = counts.iter().sum();
        if total == 0 {
            continue;
        }

        // Reward consistency: if every sampled line has the same count, the
        // separator is almost certainly structural rather than incidental.
        let min = *counts.iter().min().unwrap();
        let max = *counts.iter().max().unwrap();
        let consistency_bonus = if min == max { 2 } else { 1 };
        let score = total * consistency_bonus;

        if score > best_score {
            best_score = score;
            best_sep = sep;
        }
    }

    best_sep
}

// ── Header Detection ──────────────────────────────────────────────────────────

#[derive(Debug, PartialEq, Eq, Clone, Copy, Hash)]
enum DataType {
    Empty,
    Integer,
    Float,
    Boolean,
    String,
}

/// Infer the data type of a string cell.
fn infer_type(s: &str) -> DataType {
    let t = s.trim();
    if t.is_empty() {
        return DataType::Empty;
    }
    if t.parse::<i64>().is_ok() {
        return DataType::Integer;
    }
    if t.parse::<f64>().is_ok() {
        return DataType::Float;
    }
    if t.eq_ignore_ascii_case("true")
        || t.eq_ignore_ascii_case("false")
        || t.eq_ignore_ascii_case("yes")
        || t.eq_ignore_ascii_case("no")
    {
        return DataType::Boolean;
    }
    DataType::String
}

/// Detect if the file likely has a header row using a weighted voting system.
///
/// Reads the first few rows and compares the types of the first row (candidate header)
/// against the subsequent rows (data body).
///
/// Heuristics:
/// 1. Duplicate columns in the first row -> likely NOT a header.
/// 2. Row 0 is String, Data is Number/Bool -> Strong Vote FOR Header.
/// 3. Row 0 is Number, Data is String -> Strong Vote AGAINST Header.
/// 4. Row 0 is String, Data is String -> Weak Vote (length check).
///
/// Returns `true` by default if ambiguous or if the file is too short.
pub fn detect_header(path: &Path, sep: u8) -> bool {
    const SAMPLE_BYTES: usize = 8192;
    // Read a small sample to avoid parsing huge files.
    let content = match fs::read(path) {
        Ok(c) => c,
        Err(_) => return true,
    };
    let sample = &content[..content.len().min(SAMPLE_BYTES)];
    // Fallback to lossy UTF-8 decoding; for header detection this is usually fine.
    let (text, _, _) = decode_bytes(sample);

    let mut rdr = csv::ReaderBuilder::new()
        .delimiter(sep)
        .has_headers(false) // Read everything as data records
        .flexible(true)
        .from_reader(Cursor::new(text.as_bytes()));

    // Collect first few rows (e.g. up to 10 + 1 header)
    let mut rows: Vec<Vec<String>> = Vec::new();
    for record in rdr.records().take(11).flatten() {
        rows.push(record.iter().map(|s| s.to_string()).collect());
    }

    if rows.len() < 2 {
        return true; // Not enough data to decide, assume standard CSV.
    }

    let header_row = &rows[0];
    let body_rows = &rows[1..];
    let ncols = header_row.len();

    // 1. Immediate Disqualification: Duplicates in header row
    // Headers are almost always unique keys.
    let mut seen = std::collections::HashSet::new();
    for col in header_row {
        if !seen.insert(col) {
            return false;
        }
    }

    let mut score = 0;

    for i in 0..ncols {
        let h_val = &header_row[i];
        let h_type = infer_type(h_val);

        // Determine the "dominant" type of this column in the body
        // We use a simple majority vote among non-empty cells.
        let mut type_counts = std::collections::HashMap::new();
        let mut non_empty_count = 0;
        let mut avg_len = 0;

        for row in body_rows {
            if let Some(val) = row.get(i) {
                let d_type = infer_type(val);
                if d_type != DataType::Empty {
                    *type_counts.entry(d_type).or_insert(0) += 1;
                    non_empty_count += 1;
                    avg_len += val.len();
                }
            }
        }

        if non_empty_count == 0 {
            continue; // Skip empty columns
        }

        let avg_body_len = avg_len as f64 / non_empty_count as f64;

        // Find the most frequent type
        let (dom_type, _) = type_counts
            .iter()
            .max_by_key(|&(_, count)| count)
            .map(|(&t, &c)| (t, c))
            .unwrap_or((DataType::String, 0));

        // ── Heuristics ──

        // Strong Signal: Header is String, Body is "Strict" (Int/Float/Bool)
        if h_type == DataType::String
            && matches!(
                dom_type,
                DataType::Integer | DataType::Float | DataType::Boolean
            )
        {
            score += 5;
        }
        // Negative Signal: Header is Number, Body is String
        // (e.g. a year or ID as the first row of data, followed by names)
        else if matches!(h_type, DataType::Integer | DataType::Float)
            && dom_type == DataType::String
        {
            score -= 5;
        }
        // Weak Signal: Both are strings. Check length.
        // Headers ("Name") are often shorter than data ("The Quick Brown Fox...").
        else if h_type == DataType::String
            && dom_type == DataType::String
            && (h_val.len() as f64) < avg_body_len * 0.8
        {
            score += 1;
        }
    }

    // Default to true (has header) unless strong evidence against it.
    score >= 0
}

/// Write a CSV file in the specified encoding, atomically.
///
/// The CSV content is first written into an in-memory UTF-8 buffer, then
/// re-encoded to the target encoding.  For plain UTF-8 this is a copy; for
/// other encodings `encoding_rs` performs the conversion.  A BOM is prepended
/// when `encoding_bom` is true.
///
/// Writes to a `.csv.tmp` sidecar first, then renames atomically so a crash
/// mid-write never leaves a truncated file.
pub fn write_csv(
    path: &Path,
    sep: u8,
    has_header: bool,
    headers: &[String],
    rows: &[Vec<String>],
    encoding: &'static encoding_rs::Encoding,
    encoding_bom: bool,
) -> Result<(), CsvError> {
    let ncols = headers.len();

    // ── 1. Produce UTF-8 CSV in memory ────────────────────────────────────────
    let mut utf8_buf: Vec<u8> = Vec::new();
    {
        let mut wtr = csv::WriterBuilder::new()
            .delimiter(sep)
            .from_writer(&mut utf8_buf);

        if has_header {
            wtr.write_record(headers)?;
        }
        for row in rows {
            // Pad short rows with empty fields to keep the output rectangular.
            if row.len() < ncols {
                let mut padded = row.to_vec();
                padded.resize(ncols, String::new());
                wtr.write_record(&padded)?;
            } else {
                wtr.write_record(row)?;
            }
        }
        wtr.flush()?;
    }

    // ── 2. Re-encode to target encoding ───────────────────────────────────────
    let final_bytes = encode_output(&utf8_buf, encoding, encoding_bom);

    // ── 3. Atomic write ───────────────────────────────────────────────────────
    let tmp_path = path.with_extension("csv.tmp");
    fs::write(&tmp_path, &final_bytes)?;
    fs::rename(&tmp_path, path)?;
    Ok(())
}

/// Convert a UTF-8 byte slice to the target encoding, prepending a BOM if
/// requested.
fn encode_output(utf8: &[u8], encoding: &'static encoding_rs::Encoding, bom: bool) -> Vec<u8> {
    let bom_bytes: &[u8] = if !bom {
        b""
    } else if encoding == encoding_rs::UTF_8 {
        b"\xEF\xBB\xBF"
    } else if encoding == encoding_rs::UTF_16LE {
        b"\xFF\xFE"
    } else if encoding == encoding_rs::UTF_16BE {
        b"\xFE\xFF"
    } else {
        b""
    };

    if encoding == encoding_rs::UTF_8 {
        // Fast path: no re-encoding needed.
        let mut out = Vec::with_capacity(bom_bytes.len() + utf8.len());
        out.extend_from_slice(bom_bytes);
        out.extend_from_slice(utf8);
        out
    } else {
        let text = std::str::from_utf8(utf8).expect("csv crate produces valid UTF-8");
        let (encoded, _, _) = encoding.encode(text);
        let mut out = Vec::with_capacity(bom_bytes.len() + encoded.len());
        out.extend_from_slice(bom_bytes);
        out.extend_from_slice(&encoded);
        out
    }
}
