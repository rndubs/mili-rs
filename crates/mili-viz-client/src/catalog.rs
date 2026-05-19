//! Decoder for the result-catalog side-channel (`phase-5-m3.md`
//! Decision 67; MVP-cut item 8).
//!
//! The frozen `mili_viz.proto` carries no svar catalog, so the server
//! enumerates the loaded run's primal svars
//! (`Database::queriable_svars`) into a small self-describing blob
//! served over the existing Flight `DoGet` bulk boundary by a
//! conventional ticket (`mili_viz_server::CATALOG_TICKET`) — no
//! `.proto` change. This is the client half: a pure, GPU-free decoder
//! mirroring `mesh::decode_mvg`.
//!
//! Blob layout: the magic `MVCAT1\n`, then UTF-8 lines `TAG\tNAME`.
//! `P` = primal queriable svar. Unknown tags are skipped so a newer
//! server (adding time-indep / derived tags) degrades cleanly on an
//! older client.

/// Magic + version prefix of the catalog blob (must match
/// `mili-viz-server`'s `CATALOG_MAGIC`; independently known on each
/// side exactly like the `MVG1`/`MVG2` geometry magic).
const CATALOG_MAGIC: &[u8] = b"MVCAT1\n";

/// The decoded result catalog. Only `primal` is transported today;
/// time-independent variables have no mili-rs accessor yet, so that
/// sub-tree stays a labelled placeholder in the left dock.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ResultCatalog {
    /// Primal queriable svar names, server parse order (the griz
    /// `Results → primal` list; selecting one is the same `show
    /// <result>` the command line emits).
    pub primal: Vec<String>,
}

impl ResultCatalog {
    /// Total catalog row count, for the left-dock `Results · N` badge.
    #[must_use]
    pub fn len(&self) -> usize {
        self.primal.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.primal.is_empty()
    }
}

/// Decode a catalog blob. `None` if the magic is absent/short (a
/// non-catalog buffer); an empty primal list is still `Some` (a real
/// run that happens to expose no queriable svars).
#[must_use]
pub fn decode_catalog(blob: &[u8]) -> Option<ResultCatalog> {
    let body = blob.strip_prefix(CATALOG_MAGIC)?;
    let text = std::str::from_utf8(body).ok()?;
    let mut primal = Vec::new();
    for line in text.lines() {
        if line.is_empty() {
            continue;
        }
        let Some((tag, name)) = line.split_once('\t') else {
            continue;
        };
        if tag == "P" && !name.is_empty() {
            primal.push(name.to_string());
        }
    }
    Some(ResultCatalog { primal })
}
