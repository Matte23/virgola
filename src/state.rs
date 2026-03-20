use std::collections::HashSet;
use std::path::PathBuf;

// ── Direction ─────────────────────────────────────────────────────────────────

/// Step direction for match navigation.  Replaces the raw `i32` delta so
/// call sites are self-documenting and can't pass an arbitrary integer.
pub enum Direction {
    Prev,
    Next,
}

// ── SearchState ───────────────────────────────────────────────────────────────

/// All search-related fields, separated from the CSV data model so each can
/// be understood and tested independently.
///
/// `query` stores the *lowercased* version of the user's input — matching is
/// always case-insensitive and the lowercase form is kept to avoid re-lowering
/// on every cell comparison.
#[derive(Default)]
pub struct SearchState {
    pub query: String,
    // TODO: the HashSet and the Vec are redundant copies of the same data and
    //       must be kept manually in sync.  Consider a single `IndexSet`
    //       (indexmap crate) that provides both O(1) lookup and stable order.
    pub matches: HashSet<(usize, usize)>, // O(1) lookup for highlighting
    pub matches_ordered: Vec<(usize, usize)>, // ordered for navigation
    pub current_match: Option<usize>,     // index into matches_ordered
}

impl SearchState {
    pub fn clear(&mut self) {
        self.query.clear();
        self.matches.clear();
        self.matches_ordered.clear();
        self.current_match = None;
    }
}

// ── State ─────────────────────────────────────────────────────────────────────

pub struct State {
    pub path: Option<PathBuf>,
    pub separator: u8,
    pub headers: Vec<String>,
    pub rows: Vec<Vec<String>>,
    pub dirty: bool,
    pub search: SearchState,
}

impl Default for State {
    fn default() -> Self {
        Self {
            path: None,
            separator: b',',
            headers: Vec::new(),
            rows: Vec::new(),
            dirty: false,
            search: SearchState::default(),
        }
    }
}

impl State {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn clear_search(&mut self) {
        self.search.clear();
    }

    /// Recompute matches for `query` across all rows/cols.
    /// Returns the row of the first match (if any) for scrolling.
    pub fn update_search(&mut self, query: &str) -> Option<usize> {
        self.search.matches.clear();
        self.search.matches_ordered.clear();
        let q = query.to_lowercase();
        self.search.query = q.clone();

        if !q.is_empty() {
            for (row_idx, row) in self.rows.iter().enumerate() {
                for (col_idx, cell) in row.iter().enumerate() {
                    if cell.to_lowercase().contains(&q) {
                        self.search.matches.insert((row_idx, col_idx));
                        self.search.matches_ordered.push((row_idx, col_idx));
                    }
                }
            }
        }

        self.search.current_match = if self.search.matches_ordered.is_empty() {
            None
        } else {
            Some(0)
        };
        self.search.matches_ordered.first().map(|&(r, _)| r)
    }

    /// Step `current_match` by one position, returns the new match row.
    pub fn step_match(&mut self, dir: Direction) -> Option<usize> {
        let n = self.search.matches_ordered.len();
        if n == 0 {
            return None;
        }
        let delta: i32 = match dir {
            Direction::Prev => -1,
            Direction::Next => 1,
        };
        let cur = self.search.current_match.unwrap_or(0) as i32;
        let next = ((cur + delta).rem_euclid(n as i32)) as usize;
        self.search.current_match = Some(next);
        self.search.matches_ordered.get(next).map(|&(r, _)| r)
    }
}
