//! Fresh numeric measurements over graph-owned rows; raw diagnostics stay checked.

#[cfg(test)]
use crate::graph::root_structure::{RootNode, SharedSplit};
#[cfg(test)]
use std::collections::HashMap;

use super::*;

#[cfg(test)]
enum CheckedCompileFrame {
    Enter(NodeId),
    Exit(usize),
}

impl RootStructure {
    /// Retain malformed draft diagnostics without weakening runtime ownership.
    #[cfg(test)]
    pub(super) fn compile_draft(
        workspace: &WorkspaceData,
        root: RootId,
        leaves: LeafMeasurements<'_>,
        metrics: LayoutMetrics,
    ) -> Result<(Self, Vec<SubtreeMeasurement>), LayoutError> {
        let record = workspace
            .root(root)
            .ok_or(LayoutError::MissingRoot { root })?;
        let mut topology = Self::default();
        let mut rows = HashMap::<NodeId, usize>::new();
        let mut values = Vec::<Option<SubtreeMeasurement>>::new();
        let mut child_values = Vec::new();
        let mut pending = vec![CheckedCompileFrame::Enter(record.node)];
        while let Some(frame) = pending.pop() {
            match frame {
                CheckedCompileFrame::Enter(node) => {
                    if let Some(&row) = rows.get(&node) {
                        // A row is active until its measurement is complete.
                        if values[row].is_none() {
                            return Err(LayoutError::Cycle { node });
                        }
                        continue;
                    }
                    let live = workspace
                        .node(node)
                        .ok_or(LayoutError::MissingNode { node })?;
                    let row = topology.nodes.len();
                    topology.nodes.push(RootNode {
                        id: node,
                        kind: RootNodeKind::Tabs { children: 0..0 },
                    });
                    values.push(None);
                    rows.insert(node, row);
                    match live {
                        Node::Grid { .. } => unreachable!(
                            "the malformed-draft oracle generates only flat Tabs and Splits"
                        ),
                        Node::Pane { .. } | Node::Tabs { .. } => {
                            values[row] = Some(SubtreeMeasurement {
                                constraints: leaves.get(node)?,
                                contains_central: record.central == Some(node),
                            });
                            topology.postorder.push(row);
                        }
                        Node::Split {
                            children, weights, ..
                        } => {
                            if children.len() != weights.len() {
                                return Err(LayoutError::SplitCardinalityMismatch {
                                    node,
                                    children: children.len(),
                                    weights: weights.len(),
                                });
                            }
                            pending.push(CheckedCompileFrame::Exit(row));
                            pending.extend(
                                children
                                    .iter()
                                    .rev()
                                    .copied()
                                    .map(CheckedCompileFrame::Enter),
                            );
                        }
                    }
                }
                CheckedCompileFrame::Exit(row) => {
                    let node = topology.nodes[row].id;
                    let Some(Node::Split { axis, children, .. }) = workspace.node(node) else {
                        return Err(LayoutError::MissingNode { node });
                    };
                    child_values.clear();
                    let start = topology.children.len();
                    for child in children {
                        let child_row = *rows
                            .get(child)
                            .ok_or(LayoutError::MissingNode { node: *child })?;
                        child_values.push(
                            values[child_row].ok_or(LayoutError::MissingNode { node: *child })?,
                        );
                        topology.children.push(child_row);
                    }
                    values[row] = Some(SubtreeMeasurement {
                        constraints: compose_constraints(
                            node,
                            *axis,
                            &child_values,
                            metrics,
                            matches!(leaves, LeafMeasurements::Minimums(_)),
                        )?,
                        contains_central: child_values.iter().any(|child| child.contains_central),
                    });
                    topology.nodes[row].kind = RootNodeKind::Split {
                        payload: SharedSplit::new(Arc::clone(
                            workspace
                                .nodes
                                .shared(node)
                                .ok_or(LayoutError::MissingNode { node })?,
                        )),
                        children: start..topology.children.len(),
                        split: topology.split_rows.len(),
                    };
                    topology.split_rows.push(row);
                    topology.postorder.push(row);
                }
            }
        }
        let values = values
            .into_iter()
            .zip(&topology.nodes)
            .map(|(value, node)| value.ok_or(LayoutError::MissingNode { node: node.id }))
            .collect::<Result<Vec<_>, _>>()?;
        topology.check_central(root, record.central, &values)?;
        topology.finish_directory();
        Ok((topology, values))
    }

    pub(super) fn measure_with_reader(
        &self,
        root: RootId,
        central: Option<NodeId>,
        is_visible: &mut impl FnMut(NodeId) -> bool,
        read_leaf: &mut impl FnMut(NodeId) -> Result<Constraints, LayoutError>,
        metrics: LayoutMetrics,
        minimums_only: bool,
    ) -> Result<(Vec<NodeResult>, Option<usize>), LayoutError> {
        let zero = LogicalSize::new(0.0, 0.0)?;
        let empty = NodeResult {
            visible: false,
            slot: LogicalRect::new(0.0, 0.0, 0.0, 0.0)?,
            rect: LogicalRect::new(0.0, 0.0, 0.0, 0.0)?,
            constraints: Constraints::new(zero, zero)?,
        };
        let mut values = vec![empty; self.nodes.len()];
        let mut central_leaf_row = None;
        let mut central_reachable = central.is_none();
        let mut child_values = Vec::new();
        for &row in &self.postorder {
            let node = &self.nodes[row];
            values[row].constraints = match &node.kind {
                RootNodeKind::Pane { .. } | RootNodeKind::Tabs { .. } => {
                    if central == Some(node.id) {
                        central_reachable = true;
                    }
                    values[row].visible = is_visible(node.id);
                    if values[row].visible {
                        if central == Some(node.id) {
                            central_leaf_row = Some(row);
                        }
                        read_leaf(node.id)?
                    } else {
                        empty.constraints
                    }
                }
                RootNodeKind::Split {
                    payload, children, ..
                } => {
                    let child_rows = &self.children[children.clone()];
                    let binary_children;
                    let child_measurements: &[SubtreeMeasurement] = match child_rows {
                        [first, second] if values[*first].visible && values[*second].visible => {
                            binary_children = [
                                SubtreeMeasurement {
                                    constraints: values[*first].constraints,
                                    contains_central: false,
                                },
                                SubtreeMeasurement {
                                    constraints: values[*second].constraints,
                                    contains_central: false,
                                },
                            ];
                            &binary_children
                        }
                        _ => {
                            child_values.clear();
                            child_values.extend(child_rows.iter().filter_map(|&row| {
                                values[row].visible.then_some(SubtreeMeasurement {
                                    constraints: values[row].constraints,
                                    contains_central: false,
                                })
                            }));
                            &child_values
                        }
                    };
                    values[row].visible = !child_measurements.is_empty();
                    if child_measurements.is_empty() {
                        empty.constraints
                    } else {
                        compose_constraints(
                            node.id,
                            payload.axis(),
                            child_measurements,
                            metrics,
                            minimums_only,
                        )?
                    }
                }
                RootNodeKind::Grid {
                    payload, children, ..
                } => {
                    let children = &self.children[children.clone()];
                    let mut min_width: f64 = 0.0;
                    let mut min_height: f64 = 0.0;
                    let mut max_width: f64 = 0.0;
                    let mut max_height: f64 = 0.0;
                    let mut visible = false;
                    for &child in children {
                        if !values[child].visible {
                            continue;
                        }
                        visible = true;
                        let constraints = values[child].constraints;
                        min_width = min_width.max(constraints.min().width());
                        min_height = min_height.max(constraints.min().height());
                        max_width = max_width.max(constraints.max().width());
                        max_height = max_height.max(constraints.max().height());
                    }
                    values[row].visible = visible;
                    if let crate::model::DockspaceGridShape::Fixed { rows, columns } =
                        payload.shape()
                    {
                        grid::grid_constraints(
                            self,
                            row,
                            &values,
                            ResolvedGridShape {
                                rows: rows.get(),
                                columns: columns.get(),
                            },
                            metrics,
                            zero,
                        )?
                    } else {
                        Constraints::new(
                            LogicalSize::new(min_width, min_height)?,
                            LogicalSize::new(max_width, max_height)?,
                        )?
                    }
                }
            };
        }
        if let Some(central) = central
            && !central_reachable
        {
            return Err(LayoutError::CentralNodeNotReachable { root, central });
        }
        Ok((values, central_leaf_row))
    }

    #[cfg(test)]
    fn check_central(
        &self,
        root: RootId,
        central: Option<NodeId>,
        values: &[SubtreeMeasurement],
    ) -> Result<(), LayoutError> {
        if let Some(central) = central
            && !values[0].contains_central
        {
            return Err(LayoutError::CentralNodeNotReachable { root, central });
        }
        Ok(())
    }
}
