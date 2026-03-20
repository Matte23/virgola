use std::fmt;
use std::fs;
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

/// Returned by `read_csv`.  Carries the parsed data plus a flag that tells
/// callers whether the file had rows with a different column count than the
/// header — so the UI can warn the user without refusing to load the file.
pub struct CsvReadResult {
    pub headers: Vec<String>,
    pub rows: Vec<Vec<String>>,
    /// `true` if at least one data row had a different number of fields than
    /// the header row.
    pub had_jagged_rows: bool,
}

// ── Public API ────────────────────────────────────────────────────────────────

pub fn read_csv(path: &Path, sep: u8) -> Result<CsvReadResult, CsvError> {
    let mut rdr = csv::ReaderBuilder::new()
        .delimiter(sep)
        // flexible(true) so we don't hard-reject jagged files; we report the
        // condition via CsvReadResult::had_jagged_rows instead.
        .flexible(true)
        // TODO: no encoding is specified; the csv crate assumes UTF-8.
        //       Non-UTF-8 files (e.g. Windows-1252) will produce garbled text
        //       or an error.  Add an encoding detection / conversion step.
        .from_path(path)?;

    let headers: Vec<String> = rdr.headers()?.iter().map(|s| s.to_string()).collect();
    let ncols = headers.len();

    let mut rows: Vec<Vec<String>> = Vec::new();
    for result in rdr.records() {
        let record = result?;
        rows.push(record.iter().map(|s| s.to_string()).collect());
    }

    let had_jagged_rows = rows.iter().any(|r| r.len() != ncols);

    Ok(CsvReadResult {
        headers,
        rows,
        had_jagged_rows,
    })
}

pub fn write_csv(
    path: &Path,
    sep: u8,
    headers: &[String],
    rows: &[Vec<String>],
) -> Result<(), CsvError> {
    let ncols = headers.len();

    // Write to a temp file first, then rename atomically so a crash or error
    // mid-write never leaves a truncated/corrupt file at the target path.
    let tmp_path = path.with_extension("csv.tmp");
    {
        let mut wtr = csv::WriterBuilder::new()
            .delimiter(sep)
            .from_path(&tmp_path)?;

        wtr.write_record(headers)?;
        for row in rows {
            // Pad short rows with empty fields to keep the output rectangular.
            // Rows that are already the right length or longer are written as-is
            // (extra fields are preserved — the user put them there).
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
    fs::rename(&tmp_path, path)?;
    Ok(())
}
