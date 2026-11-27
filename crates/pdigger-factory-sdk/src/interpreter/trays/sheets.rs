use super::*;
use serde::ser::{SerializeStruct, Serializer};

// Hardware coordinates for both board continuity checks. Invalid or incomplete outlines carry no evidence.
pub(super) fn extent(points: &[Vec<f64>]) -> Option<[f64; 4]> {
    if points.len() < 3 {
        return None;
    }
    let mut b = [f64::INFINITY, f64::INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY];
    for p in points {
        let &[x, y] = p.as_slice() else {
            return None;
        };
        if !x.is_finite() || !y.is_finite() {
            return None;
        }
        b = [b[0].min(x), b[1].min(y), b[2].max(x), b[3].max(y)];
    }
    Some(b)
}

impl Ws {
    fn inside_tray(&self, b: [f64; 4]) -> bool {
        self.mode == StoneMode::Seen
            && self.detected
            && !self.under_sheet
            && self.outline.is_some_and(|[l, t, r, d]| l >= b[0] && t >= b[1] && r <= b[2] && d <= b[3])
    }

    fn retired_outside(&self, b: [f64; 4]) -> bool {
        self.mode == StoneMode::Kept
            && !self.detected
            && !self.under_sheet
            && self.outline.is_some_and(|[l, t, r, d]| r < b[0] || l > b[2] || d < b[1] || t > b[3])
    }
}

impl TrayLedger {
    pub(super) fn only_background_retired(&self, tr: &OpenTray, tw: &TrayWorld, hole: bool) -> bool {
        let (Some(b), Some(prev)) = (self.bounds, tr.last.as_ref()) else { return false };
        if tr.arrived.is_some() || hole || prev.r.g.checked_add(1) != Some(tw.r.g) {
            return false;
        }
        let disturbed = tw.events.iter().any(|e| match e.kind {
            EventKind::New | EventKind::Split => true,
            EventKind::Delete => {
                !matches!(e.why.as_deref(), Some("stranded" | "outside"))
                    || !e.id.is_some_and(|id| {
                        !self.visit_seen.contains(&id) && prev.get(id).is_some_and(|s| s.retired_outside(b))
                    })
            }
            _ => false,
        });
        if disturbed {
            return false;
        }
        tw.stones
            .iter()
            .filter(|w| {
                w.inside_tray(b)
                    && prev.get(w.id).is_some_and(|p| p.detected && !p.under_sheet && dist(p.pos, w.pos) <= NEAR)
            })
            .count()
            >= 2
    }

    pub(super) fn retire_sheet(&mut self, tw: &TrayWorld, stationary: bool, hole: bool, sink: &mut Sink<'_>) -> bool {
        let Some(tr) = self.tray.as_mut().filter(|tr| tr.arrived.is_some() && tr.last.is_some()) else { return false };
        let retired: BTreeSet<_> = tw
            .events
            .iter()
            .filter(|e| e.kind == EventKind::Delete && e.why.as_deref() == Some("sheet"))
            .filter_map(|e| e.id)
            .collect();
        if retired.is_empty() {
            return false;
        }
        let lift: BTreeSet<_> = tw
            .events
            .iter()
            .filter(|e| e.kind == EventKind::Sheet && e.why.as_deref() == Some("lifted"))
            .flat_map(|e| e.ids.iter().copied())
            .collect();
        let mut affected = Vec::new();
        let mut lifted = Vec::new();
        let mut anchors = 0;
        for (&id, &i) in &tr.link {
            let s = &tr.stones[i];
            if !s.under_sheet {
                continue;
            }
            if lift.contains(&id) {
                lifted.push(i);
            }
            if retired.contains(&id) {
                affected.push((id, i));
            } else if tw.get(id).is_some_and(|w| w.under_sheet && dist(s.place, w.pos) <= NEAR) {
                anchors += 1;
            }
        }
        if affected.is_empty() {
            return false;
        }
        if self.on
            && !hole
            && stationary
            && affected.iter().all(|(id, _)| lift.contains(id))
            && (anchors >= 2 || self.bounds.is_some_and(|b| tr.same_exposed_layer(tw, b)))
        {
            let stones: Vec<_> = affected.iter().map(|&(_, i)| i).collect();
            tr.sheet_laid(&stones, tw.r, sink);
            tr.sheet_fact(&stones, "sheet_lifted", tw.r, sink);
            for (id, i) in affected {
                tr.link.remove(&id);
                tr.lose(i, tw.r);
                tr.stones[i].sheet_loss = true;
            }
            tr.worked = true;
            return false;
        }
        self.close_at(Some(tw.r), &lifted, sink);
        self.stop.clear();
        self.on = false;
        true
    }
}

impl OpenTray {
    fn same_exposed_layer(&self, tw: &TrayWorld, b: [f64; 4]) -> bool {
        let Some(prev) = self.last.as_ref().filter(|p| p.r.g.checked_add(1) == Some(tw.r.g)) else { return false };
        let mut before = 0;
        let mut same = 0;
        for (&id, &i) in &self.link {
            let Some(p) = prev.get(id).filter(|p| !self.stones[i].under_sheet && p.inside_tray(b)) else { continue };
            before += 1;
            if tw.get(id).is_some_and(|w| w.inside_tray(b) && dist(w.pos, p.pos) <= NEAR) {
                same += 1;
            }
        }
        same >= 2 && 2 * same > before
    }

    pub(super) fn sheet_fact(&mut self, stones: &[usize], code: &str, r: TimeRef, sink: &mut Sink<'_>) {
        if stones.is_empty() {
            return;
        }
        let mut names: Vec<_> = stones.iter().map(|&i| self.stones[i].id.clone()).collect();
        names.sort();
        let fact = TrayFact {
            stones: Some(names.len() as u64),
            tray_stones: names,
            code: Some(code.to_owned()),
            ..self.fact(TrayFactKind::Sheet, None, r)
        };
        self.say(fact, sink);
    }

    pub(super) fn sheet_laid(&mut self, stones: &[usize], r: TimeRef, sink: &mut Sink<'_>) {
        let newly: Vec<_> = stones
            .iter()
            .copied()
            .filter(|&i| self.stones[i].under_sheet && self.stones[i].sheet_since.is_none())
            .collect();
        if !newly.is_empty() {
            for &i in &newly {
                self.stones[i].sheet_since = Some(r);
            }
            sink.counts.sheets += 1;
            self.sheet_fact(&newly, "sheet_laid", r, sink);
        }
    }

    pub(super) fn observe_sheets(&mut self, tw: &TrayWorld, calm: bool, sink: &mut Sink<'_>) {
        let mut clear = Vec::new();
        for (&id, &i) in &self.link {
            let Some(w) = tw.get(id) else { continue };
            if w.under_sheet {
                self.stones[i].under_sheet = true;
            } else if self.stones[i].under_sheet {
                clear.push(i);
            }
        }
        self.sheet_laid(&clear, tw.r, sink);
        self.sheet_fact(&clear, "sheet_lifted", tw.r, sink);
        for i in clear {
            self.stones[i].under_sheet = false;
            self.stones[i].sheet_since = None;
        }
        if calm {
            self.sheet_laid(&(0..self.stones.len()).collect::<Vec<_>>(), tw.r, sink);
        }
    }
}

pub(super) fn check_sheets<'a>(
    t: &'a Tray,
    by_id: &BTreeMap<&str, &TrayStone>,
    bad: &mut BTreeSet<&'static str>,
) -> BTreeSet<&'a str> {
    let mut covered = BTreeSet::new();
    let mut lifted = BTreeSet::new();
    for f in t.facts.iter().filter(|f| f.kind == TrayFactKind::Sheet) {
        if f.tray_stones.is_empty()
            || f.stones != Some(f.tray_stones.len() as u64)
            || f.tray_stones.windows(2).any(|p| p[0] >= p[1])
        {
            bad.insert("C10");
        }
        for id in &f.tray_stones {
            if by_id.get(id.as_str()).is_none_or(|s| s.born.g > f.g) {
                bad.insert("C10");
            }
            let valid = match f.code.as_deref() {
                Some("sheet_laid") => covered.insert(id.as_str()),
                Some("sheet_lifted") => {
                    lifted.insert(id.as_str());
                    covered.remove(id.as_str())
                }
                _ => false,
            };
            if !valid {
                bad.insert("C10");
            }
        }
    }
    if let Some(f) = t.facts.last().filter(|f| f.kind == TrayFactKind::Leaving) {
        let under = t.stones.iter().filter(|s| s.status == TrayStoneStatus::On && s.under_sheet).count() as u64;
        if f.under_sheet.unwrap_or(0) != under || under > f.stones.unwrap_or(0) {
            bad.insert("C11");
        }
    }
    lifted
}

impl Serialize for TrayCounts {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        let has_paper = self.sheets > 0 || self.under_sheet > 0;
        let mut m = s.serialize_struct("TrayCounts", 8 + if has_paper { 2 } else { 0 })?;
        m.serialize_field("total", &self.total)?;
        m.serialize_field("arrived", &self.arrived)?;
        m.serialize_field("stones", &self.stones)?;
        m.serialize_field("added", &self.added)?;
        m.serialize_field("added_unclear", &self.added_unclear)?;
        m.serialize_field("removed", &self.removed)?;
        m.serialize_field("removed_unclear", &self.removed_unclear)?;
        m.serialize_field("unsure", &self.unsure)?;
        if has_paper {
            m.serialize_field("sheets", &self.sheets)?;
            m.serialize_field("under_sheet", &self.under_sheet)?;
        }
        m.end()
    }
}

impl Serialize for TrayReport {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        let has_paper = !self.under_sheet.is_empty() || !self.sheet_facts.is_empty();
        let mut m = s.serialize_struct("TrayReport", 12 + if has_paper { 3 } else { 0 })?;
        m.serialize_field("session", &self.session)?;
        m.serialize_field("video", &self.video)?;
        m.serialize_field("tray", &self.tray)?;
        m.serialize_field("arrived", &self.arrived)?;
        m.serialize_field("left", &self.left)?;
        m.serialize_field("stones", &self.stones)?;
        m.serialize_field("removed", &self.removed)?;
        m.serialize_field("added", &self.added)?;
        m.serialize_field("moved", &self.moved)?;
        m.serialize_field("unclear", &self.unclear)?;
        m.serialize_field("unsure", &self.unsure)?;
        m.serialize_field("complete", &self.complete)?;
        if has_paper {
            m.serialize_field("under_sheet", &self.under_sheet)?;
            m.serialize_field("sheets", &self.sheets)?;
            m.serialize_field("sheet_facts", &self.sheet_facts)?;
        }
        m.end()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_complete_finite_outlines_have_bounds() {
        assert_eq!(extent(&[]), None);
        assert_eq!(extent(&[vec![1., 1.], vec![], vec![2., 2.]]), None);
        assert_eq!(extent(&[vec![1., 1.], vec![f64::INFINITY, 3.], vec![2., 2.]]), None);
        assert_eq!(extent(&[vec![1., 5.], vec![3., 1.], vec![2., 2.]]), Some([1., 1., 3., 5.]));
    }
}
