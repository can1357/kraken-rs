use std::collections::{HashMap, HashSet};

use num_traits::ToPrimitive;

use crate::git::models::CommitSummary;

/// One colored lane segment spanning a commit row to the next row.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct LaneSegment {
    pub(crate) from: usize,
    pub(crate) to: usize,
    pub(crate) color: usize,
}

/// Precomputed graph geometry metadata for one commit row.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct GraphRow {
    /// Horizontal lane containing the commit node.
    pub(crate) lane: usize,
    /// Palette index carried by the node and its first-parent edge.
    pub(crate) color: usize,
    /// Marks the start of a same-author run in this lane; later commits of
    /// the run render as small dots instead of avatar nodes.
    pub(crate) show_avatar: bool,
    /// Lane edges leaving this row toward the next visible commit.
    pub(crate) segments: Vec<LaneSegment>,
}

/// Incremental topological lane assignment consumed by the virtualized graph view.
#[derive(Clone, Debug, Default)]
pub(crate) struct GraphLayout {
    pub(crate) rows: Vec<GraphRow>,
    pub(crate) max_lanes: usize,
}

impl GraphLayout {
    /// Assigns stable lanes and merge transitions for topologically ordered commits.
    ///
    /// GitKraken-style rules: lane 0 is reserved for the checked-out branch's
    /// first-parent chain, a commit takes the leftmost lane expecting it,
    /// every duplicate expectation terminates with an elbow into its node at
    /// the junction row, the first parent inherits the commit's lane, and
    /// extra parents take free lanes to its right.
    pub(crate) fn build(commits: &[CommitSummary]) -> Self {
        let index_of: HashMap<&str, usize> = commits
            .iter()
            .enumerate()
            .map(|(index, commit)| (commit.id.as_str(), index))
            .collect();
        let trunk = trunk_ids(commits, &index_of);
        let reserved = usize::from(!trunk.is_empty());

        let mut active: Vec<Option<&str>> = Vec::new();
        let mut lane_author: HashMap<usize, &str> = HashMap::new();
        let mut rows: Vec<GraphRow> = Vec::with_capacity(commits.len());
        let mut max_lanes = 1;

        for (row_index, commit) in commits.iter().enumerate() {
            let id = commit.id.as_str();
            let lane = match active.iter().position(|entry| *entry == Some(id)) {
                Some(expecting) => expecting,
                // The checked-out tip (or a trunk commit whose chain has not
                // been reached yet) is pinned to the reserved left edge.
                None if trunk.contains(id) => {
                    if active.is_empty() {
                        active.push(None);
                    }
                    0
                }
                None => allocate_lane(&mut active, id, reserved),
            };

            // A shared ancestor keeps the leftmost incoming lane. Every other
            // expectation bends into that node on the preceding row.
            for other in 0..active.len() {
                if other == lane || active[other] != Some(id) {
                    continue;
                }
                active[other] = None;
                lane_author.remove(&other);
                if let Some(previous) = row_index.checked_sub(1) {
                    for segment in &mut rows[previous].segments {
                        if segment.to == other {
                            segment.to = lane;
                        }
                    }
                }
            }
            // GitKraken collapses same-author runs within a lane: the avatar
            // marks the start of a run, later commits render as small dots.
            let show_avatar = lane_author.get(&lane) != Some(&commit.author.as_str());
            lane_author.insert(lane, commit.author.as_str());

            let mut segments = active
                .iter()
                .enumerate()
                .filter_map(|(index, entry)| {
                    (index != lane && entry.is_some()).then_some(LaneSegment {
                        from: index,
                        to: index,
                        color: index,
                    })
                })
                .collect::<Vec<_>>();

            if commit.parents.is_empty() {
                active[lane] = None;
                lane_author.remove(&lane);
            } else {
                let first = commit.parents[0].as_str();
                // The first-parent edge continues down the node's lane. If
                // several lanes reach the same ancestor, they converge there.
                active[lane] = Some(first);
                segments.push(LaneSegment {
                    from: lane,
                    to: lane,
                    color: lane,
                });

                for parent in commit.parents.iter().skip(1) {
                    let parent = parent.as_str();
                    let target = active
                        .iter()
                        .position(|entry| *entry == Some(parent))
                        .unwrap_or_else(|| {
                            allocate_lane(&mut active, parent, (lane + 1).max(reserved))
                        });
                    segments.push(LaneSegment {
                        from: lane,
                        to: target,
                        color: target,
                    });
                }
            }

            while active.last().is_some_and(Option::is_none) {
                active.pop();
            }
            max_lanes = max_lanes.max(active.len()).max(lane + 1);
            rows.push(GraphRow {
                lane,
                color: lane,
                show_avatar,
                segments,
            });
        }
        Self { rows, max_lanes }
    }

    /// Returns the commit indices intersecting a scrolled viewport plus one guard row.
    pub(crate) fn visible_range(
        &self,
        scroll: f32,
        viewport_height: f32,
        row_height: f32,
    ) -> std::ops::Range<usize> {
        if self.rows.is_empty() || row_height <= 0.0 {
            return 0..0;
        }
        let start = (scroll.max(0.0) / row_height)
            .floor()
            .to_usize()
            .unwrap_or(0)
            .min(self.rows.len());
        let visible = (viewport_height.max(0.0) / row_height)
            .ceil()
            .to_usize()
            .unwrap_or(0)
            .saturating_add(2);
        start..start.saturating_add(visible).min(self.rows.len())
    }

    /// Computes the final scroll offset, including rows outside the commit model.
    pub(crate) fn max_scroll(
        &self,
        viewport_height: f32,
        row_height: f32,
        extra_rows: usize,
    ) -> f32 {
        let rows = self.rows.len().saturating_add(extra_rows);
        let content = rows.to_f32().unwrap_or(f32::MAX) * row_height;
        (content - viewport_height).max(0.0)
    }
}

/// Claims the lowest free lane at or beyond `start` for an expected commit.
fn allocate_lane<'a>(active: &mut Vec<Option<&'a str>>, target: &'a str, start: usize) -> usize {
    if let Some(offset) = active.iter().skip(start).position(Option::is_none) {
        let index = start + offset;
        active[index] = Some(target);
        index
    } else {
        let index = active.len().max(start);
        active.resize(index, None);
        active.push(Some(target));
        index
    }
}

/// Collects the first-parent ancestry of the checked-out HEAD tip, which owns
/// the reserved leftmost lane.
fn trunk_ids<'a>(
    commits: &'a [CommitSummary],
    index_of: &HashMap<&'a str, usize>,
) -> HashSet<&'a str> {
    let mut trunk = HashSet::new();
    let Some(tip) = commits.iter().find(|commit| {
        commit
            .branch_refs
            .iter()
            .any(|reference| reference.is_head && !reference.is_tag)
    }) else {
        return trunk;
    };
    let mut cursor = Some(tip);
    while let Some(commit) = cursor {
        if !trunk.insert(commit.id.as_str()) {
            break;
        }
        cursor = commit
            .parents
            .first()
            .and_then(|parent| index_of.get(parent.as_str()))
            .map(|&index| &commits[index]);
    }
    trunk
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::git::models::CommitSummary;

    fn commit(id: &str, parents: &[&str]) -> CommitSummary {
        CommitSummary {
            id: id.to_owned(),
            short_id: id.to_owned(),
            subject: id.to_owned(),
            description: String::new(),
            author: "tester".to_owned(),
            email: "tester@example.com".to_owned(),
            authored_seconds: 0,
            parents: parents.iter().map(|parent| (*parent).to_owned()).collect(),
            is_local: false,
            refs: Vec::new(),
            branch_refs: Vec::new(),
        }
    }

    #[test]
    fn merge_parent_gets_a_distinct_lane_and_converges() {
        let commits = [
            commit("m", &["a", "b"]),
            commit("b", &["c"]),
            commit("a", &["c"]),
            commit("c", &[]),
        ];
        let layout = GraphLayout::build(&commits);
        assert_eq!(layout.rows[0].lane, 0);
        assert!(layout.rows[0].segments.iter().any(|edge| edge.to == 1));
        assert_eq!(layout.rows[1].lane, 1);
        assert_eq!(layout.rows[2].lane, 0);
        assert_eq!(layout.rows[3].lane, 0);
        assert!(layout.max_lanes >= 2);
    }

    #[test]
    fn checked_out_first_parent_chain_is_pinned_to_lane_zero() {
        let mut head = commit("h", &["t1"]);
        head.branch_refs.push(crate::git::models::CommitBranchRef {
            branch_short_name: "main".to_owned(),
            is_local: true,
            remote_names: Vec::new(),
            is_head: true,
            is_tag: false,
        });
        let commits = [
            commit("f", &["t2"]),
            head,
            commit("t1", &["t2"]),
            commit("t2", &[]),
        ];
        let layout = GraphLayout::build(&commits);
        // The feature branch above HEAD stays out of the reserved lane.
        assert_eq!(layout.rows[0].lane, 1);
        // HEAD's first-parent chain hugs lane 0, straight.
        assert_eq!(layout.rows[1].lane, 0);
        assert_eq!(layout.rows[2].lane, 0);
        assert_eq!(layout.rows[3].lane, 0);
        // The feature edge elbows into t2's node instead of dangling.
        assert!(layout.rows[2].segments.contains(&LaneSegment {
            from: 1,
            to: 0,
            color: 1,
        }));
    }

    #[test]
    fn expecting_lanes_terminate_into_the_shared_node() {
        // Two branches converge on the same parent; each keeps its own lane
        // all the way down and the right lane's final edge elbows into the
        // shared node at the junction row (GitKraken routing).
        let commits = [
            commit("m", &["a", "b"]),
            commit("b", &["c"]),
            commit("a", &["c"]),
            commit("c", &[]),
        ];
        let layout = GraphLayout::build(&commits);
        // The shared parent lands on the leftmost expecting lane, and the row
        // above carries the merge elbow from the terminating right lane.
        assert_eq!(layout.rows[3].lane, 0);
        assert!(
            layout.rows[2]
                .segments
                .iter()
                .any(|edge| edge.from == 1 && edge.to == 0)
        );
    }

    #[test]
    fn viewport_never_lays_out_the_full_history() {
        let commits = (0..10_000)
            .map(|index| {
                let parent = (index + 1 < 10_000).then(|| (index + 1).to_string());
                CommitSummary {
                    id: index.to_string(),
                    short_id: index.to_string(),
                    subject: index.to_string(),
                    description: String::new(),
                    author: "tester".to_owned(),
                    authored_seconds: 0,
                    email: "tester@example.com".to_owned(),
                    parents: parent.into_iter().collect(),
                    is_local: false,
                    refs: Vec::new(),
                    branch_refs: Vec::new(),
                }
            })
            .collect::<Vec<_>>();
        let layout = GraphLayout::build(&commits);
        let visible = layout.visible_range(220_000.0, 900.0, 24.0);
        assert!(visible.len() < 50);
        assert!(visible.start > 9_000);
        assert_eq!(layout.max_lanes, 1);
        assert!((layout.max_scroll(900.0, 24.0, 2) - 239_148.0).abs() < f32::EPSILON);
    }
}
