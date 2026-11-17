//! Exact replay of geometry, independent of the output capability that binds it.

use super::*;
use crate::presentation::scene::PaneRecord;

#[cfg(test)]
mod tests;

#[derive(Debug)]
pub(super) struct HitGeometry {
    pub(super) regions: Box<[PresentationHitRegion]>,
    // Fragment rows refer only to this immutable region arena.
    pub(super) touch_regions: Box<[(usize, LogicalRect)]>,
    // A private replay hint, unavailable after compaction removes any row.
    pub(super) emission_rows: Option<Box<[usize]>>,
    // A spatial leaf permutation, independent from all output authority.
    pub(super) touch_order: Box<[usize]>,
}

impl PartialEq for HitGeometry {
    fn eq(&self, other: &Self) -> bool {
        self.regions == other.regions && self.touch_regions == other.touch_regions
    }
}

pub(super) fn rect_bits(rect: LogicalRect) -> [u64; 4] {
    [rect.x(), rect.y(), rect.max().x(), rect.max().y()].map(f64::to_bits)
}

fn same_region(left: &PresentationHitRegion, right: &PresentationHitRegion) -> bool {
    left.id == right.id
        && rect_bits(left.hit.rect()) == rect_bits(right.hit.rect())
        && rect_bits(left.anchor) == rect_bits(right.anchor)
        && left.lanes == right.lanes
        && left.stack == right.stack
        && left.behavior == right.behavior
}

fn directly_selected_pane(plan: &PresentationPlan, id: PaneSceneId) -> Option<&PaneRecord> {
    let projection = plan.layout_facts()?.root(id.root)?.projection();
    if projection.nodes.topology_has_composition() {
        return None;
    }
    let row = plan
        .pane_records()
        .binary_search_by_key(&id, PaneRecord::id)
        .ok()?;
    let pane = &plan.pane_records()[row];
    let bar = pane.containing_tab_bar()?;
    if bar.root != id.root
        || bar.tabs != pane.owner_node()
        || plan.selected_in_tabs(bar.tabs) != Some(id.item.into())
    {
        return None;
    }
    let mut ancestry = plan.receiver_active_ancestry(PresentationHitRegionKind::PaneBody(id))?;
    let choice = ancestry.next()?;
    (choice.tabs() == bar.tabs && choice.selected() == id.item.into() && ancestry.next().is_none())
        .then_some(pane)
}

fn same_selection_geometry(
    plan: &PresentationPlan,
    previous_plan: &PresentationPlan,
    region: &PresentationHitRegion,
    previous: &PresentationHitRegion,
) -> bool {
    let (PresentationHitRegionKind::PaneBody(id), PresentationHitRegionKind::PaneBody(previous_id)) =
        (region.id.kind(), previous.id.kind())
    else {
        return false;
    };
    if region.id.surface() != previous.id.surface() || id.root != previous_id.root {
        return false;
    }
    let Some(pane) = directly_selected_pane(plan, id) else {
        return false;
    };
    let Some(previous_pane) = directly_selected_pane(previous_plan, previous_id) else {
        return false;
    };
    if pane.owner_node() != previous_pane.owner_node()
        || pane.containing_tab_bar() != previous_pane.containing_tab_bar()
        || pane.active_ancestry() != previous_pane.active_ancestry()
        || rect_bits(pane.slot()) != rect_bits(previous_pane.slot())
        || rect_bits(pane.bounds()) != rect_bits(previous_pane.bounds())
        || rect_bits(pane.clip()) != rect_bits(previous_pane.clip())
        || pane.layer() != previous_pane.layer()
    {
        return false;
    }
    // Only the freshly proved content identity changes. All hit geometry and
    // receiver permissions still pass the exact, identity-sensitive comparison.
    let mut rebound = *previous;
    rebound.id = region.id;
    same_region(region, &rebound)
}

impl HitGeometry {
    fn rebind_selection_regions(
        &self,
        replacements: Vec<(usize, PresentationHitRegion)>,
    ) -> Option<Self> {
        let mut regions = self.regions.to_vec();
        for (row, region) in replacements {
            *regions.get_mut(row)? = region;
        }
        // These input rows were previously canonical, not in emission order.
        // A new item can move past other pane IDs, so both mappings must advance.
        let canonical_rows = region_order::canonicalize_regions(&mut regions);
        if regions.windows(2).any(|pair| pair[0].id == pair[1].id)
            || self
                .touch_regions
                .iter()
                .any(|(row, _)| canonical_rows.get(*row) != Some(row))
        {
            return None;
        }
        let emission_rows = self
            .emission_rows
            .as_ref()?
            .iter()
            .map(|&row| canonical_rows.get(row).copied())
            .collect::<Option<Box<[_]>>>()?;
        // PaneBody contributes neither touch allocations nor protected chrome.
        // Its permutation leaves every other kind's canonical row unchanged.
        Some(Self {
            regions: regions.into_boxed_slice(),
            touch_regions: self.touch_regions.clone(),
            emission_rows: Some(emission_rows),
            touch_order: self.touch_order.clone(),
        })
    }
}

impl PresentationHitManifest {
    pub(crate) fn compile_reusing(
        output: SurfacePresentationOutputTicket,
        plan: &PresentationPlan,
        previous: Option<(&PresentationPlan, &Self)>,
    ) -> Result<Self, SceneBuildError> {
        debug_assert_eq!(output.surface(), plan.surface());
        if let Some((previous_plan, previous)) = previous
            && let Some(emission_rows) = &previous.geometry.emission_rows
            // Resize is common and must reject before traversing either inventory.
            && rect_bits(plan.bounds()) == rect_bits(previous_plan.bounds())
            && touch::same_context(plan, previous_plan)
        {
            let mut accepted_count = 0;
            let mut reusable = true;
            let mut replacements = Vec::new();
            emit_regions(plan, &mut |region| {
                let previous_region = emission_rows
                    .get(accepted_count)
                    .and_then(|&row| previous.geometry.regions.get(row).map(|old| (row, old)));
                if reusable {
                    reusable = match previous_region {
                        Some((_, old)) if same_region(&region, old) => true,
                        Some((row, old))
                            if same_selection_geometry(plan, previous_plan, &region, old) =>
                        {
                            replacements.push((row, region));
                            true
                        }
                        _ => false,
                    };
                }
                accepted_count += 1;
            })?;
            if reusable && accepted_count == emission_rows.len() {
                if replacements.is_empty() {
                    return Ok(Self {
                        output,
                        geometry: Arc::clone(&previous.geometry),
                    });
                }
                if let Some(geometry) = previous.geometry.rebind_selection_regions(replacements) {
                    return Ok(Self {
                        output,
                        geometry: Arc::new(geometry),
                    });
                }
            }
        }
        Self::compile_with_order(
            output,
            plan,
            previous.map(|(_, manifest)| &*manifest.geometry),
        )
    }
}
