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
//! `P` = primal queriable svar, `D` = computable derived result
//! (`phase-5-m4.md` Decision 71), `M` = per-element-class membership
//! `M\t<class_idx>\t<class_name>\t<labels.csv>` (wireframe-parity #6
//! path (a) — resolves a picked tri's `member_id` to `class · label`).
//! Unknown tags (e.g. a future `T` time-indep) are skipped so a newer
//! server degrades cleanly on an older client.

/// Magic + version prefix of the catalog blob (must match
/// `mili-viz-server`'s `CATALOG_MAGIC`; independently known on each
/// side exactly like the `MVG1`/`MVG2` geometry magic).
const CATALOG_MAGIC: &[u8] = b"MVCAT1\n";

/// The decoded result catalog. `primal` and `derived` are transported;
/// time-independent variables have no mili-rs accessor yet
/// (`phase-5-m4.md` Decision 69), so that sub-tree stays a labelled
/// placeholder in the left dock.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ResultCatalog {
    /// Primal queriable svar names, server parse order (the griz
    /// `Results → primal` list; selecting one is the same `show
    /// <result>` the command line emits).
    pub primal: Vec<String>,
    /// Computable derived result names — the DB-filtered union the
    /// server enumerates via `Database::derived_variables_of_class`
    /// (`phase-5-m4.md` Decision 71); same `Show` semantics as
    /// `primal`. Replaces the static `DERIVED_RESULTS` fallback once a
    /// real run is attached.
    pub derived: Vec<String>,
    /// Per-element-class membership rows (wireframe-parity #6 path
    /// (a)). `classes[k].class_idx` is dense from 0 in build order
    /// (matches the high 8 bits of the geometry blob's per-tri
    /// `tri_member_id`); `labels[elem_row]` resolves the low 24 bits
    /// back to the user-facing element label.
    pub classes: Vec<ClassMembership>,
}

/// One row of the catalog `M`-tag table: the labels (in element-row
/// order) belonging to a single element class.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ClassMembership {
    /// Index matching the high 8 bits of the geometry blob's
    /// `tri_member_id` packing.
    pub class_idx: u32,
    /// Short class name (`brick`, `sand`, …).
    pub name: String,
    /// User-facing element labels in element-row order.
    pub labels: Vec<i32>,
}

impl ResultCatalog {
    /// Total catalog row count, for the left-dock `Results · N` badge.
    #[must_use]
    pub fn len(&self) -> usize {
        self.primal.len() + self.derived.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.primal.is_empty() && self.derived.is_empty()
    }

    /// Resolve a `tri_member_id` packed `(class_idx, elem_row)` to
    /// `(class_name, label)` via the `M`-tag membership table.
    /// `None` if the catalog has no entry for that `class_idx` or
    /// the `elem_row` is out of range. Sentinel
    /// `mesh::TRI_MEMBER_NONE` is the caller's responsibility — pass
    /// `Option<u32>` from `Pick::member_id` directly to elide the
    /// sentinel branch.
    #[must_use]
    pub fn resolve_member(&self, member_id: u32) -> Option<(&str, i32)> {
        let class_idx = member_id >> 24;
        let elem_row = (member_id & 0x00FF_FFFF) as usize;
        let class = self.classes.iter().find(|c| c.class_idx == class_idx)?;
        let label = class.labels.get(elem_row).copied()?;
        Some((class.name.as_str(), label))
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
    let mut derived = Vec::new();
    let mut classes = Vec::new();
    for line in text.lines() {
        if line.is_empty() {
            continue;
        }
        let Some((tag, rest)) = line.split_once('\t') else {
            continue;
        };
        match tag {
            "P" => {
                if !rest.is_empty() {
                    primal.push(rest.to_string());
                }
            }
            "D" => {
                if !rest.is_empty() {
                    derived.push(rest.to_string());
                }
            }
            "M" => {
                if let Some(m) = parse_member_row(rest) {
                    classes.push(m);
                }
                // Malformed M rows drop silently — consistent with
                // the rest of the unknown-tag tolerance below.
            }
            _ => {} // unknown (e.g. future `T`) — degrade cleanly
        }
    }
    Some(ResultCatalog {
        primal,
        derived,
        classes,
    })
}

/// Parse the trailing fields of an `M\t` row (everything after the
/// leading `M\t`): `<class_idx>\t<name>\t<label0>,<label1>,...`.
/// Empty labels list is rejected so the dock doesn't show a class
/// with no elements.
fn parse_member_row(rest: &str) -> Option<ClassMembership> {
    let mut parts = rest.splitn(3, '\t');
    let class_idx: u32 = parts.next()?.parse().ok()?;
    let name = parts.next()?.to_string();
    if name.is_empty() {
        return None;
    }
    let labels_csv = parts.next()?;
    if labels_csv.is_empty() {
        return None;
    }
    let mut labels = Vec::new();
    for s in labels_csv.split(',') {
        labels.push(s.parse::<i32>().ok()?);
    }
    if labels.is_empty() {
        return None;
    }
    Some(ClassMembership {
        class_idx,
        name,
        labels,
    })
}
