//! Where text lives in a top-level shape, for text edits ([`super::text`]): its own body, each
//! cell of a table, and the same for every shape inside a group (by the inner shape's id).

use serde::{Deserialize, Serialize};

use deckcraft_model::text::Paragraph;
use deckcraft_model::{Shape, ShapeId, ShapeKind, find_shape_mut};

/// Groups nested deeper than this are sent whole.
const MAX_DEPTH: usize = 64;

/// One place text lives in a top-level shape.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(default)]
pub struct Part {
    /// A shape inside the group (`None`: the top-level shape itself).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub child: Option<ShapeId>,
    /// A table cell (row, column) of that shape (`None`: its own text).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cell: Option<(usize, usize)>,
}

/// Every place text lives in `sh`, in a fixed order, with its paragraphs.
pub fn parts(sh: &Shape) -> Vec<(Part, &[Paragraph])> {
    let mut out = Vec::new();
    visit(sh, None, 0, &mut out);
    out
}

fn visit<'a>(sh: &'a Shape, child: Option<ShapeId>, depth: usize, out: &mut Vec<(Part, &'a [Paragraph])>) {
    if let Some(t) = &sh.text {
        out.push((Part { child, cell: None }, &t.paragraphs));
    }
    if let ShapeKind::Table(table) = &sh.kind {
        for (r, row) in table.rows.iter().enumerate() {
            for (c, cell) in row.cells.iter().enumerate() {
                out.push((Part { child, cell: Some((r, c)) }, &cell.text.paragraphs));
            }
        }
    }
    if depth < MAX_DEPTH {
        for ch in sh.children() {
            visit(ch, Some(ch.id), depth + 1, out);
        }
    }
}

/// The paragraphs at `part` of `sh`, to change.
pub fn part_mut<'a>(sh: &'a mut Shape, part: &Part) -> Option<&'a mut Vec<Paragraph>> {
    let target = match part.child {
        None => sh,
        Some(id) => find_shape_mut(sh.children_mut()?, id)?,
    };
    match part.cell {
        None => target.text.as_mut().map(|t| &mut t.paragraphs),
        Some((r, c)) => match &mut target.kind {
            ShapeKind::Table(table) => table.rows.get_mut(r)?.cells.get_mut(c).map(|cell| &mut cell.text.paragraphs),
            _ => None,
        },
    }
}

/// `sh` with every paragraph of its text taken out: what else about it changed.
pub fn bare(sh: &Shape) -> Shape {
    let keys: Vec<Part> = parts(sh).into_iter().map(|(p, _)| p).collect();
    let mut sh = sh.clone();
    for part in keys {
        if let Some(paras) = part_mut(&mut sh, &part) {
            paras.clear();
        }
    }
    sh
}

/// Where the top-level shape among `shapes` that holds the shape `id` is, and the [`Part`] that
/// is that shape's own text (cell `cell`).
pub fn locate(shapes: &[Shape], id: ShapeId, cell: Option<(usize, usize)>) -> Option<(ShapeId, Part)> {
    for top in shapes {
        if top.id == id {
            return Some((top.id, Part { child: None, cell }));
        }
        if deckcraft_model::find_shape(top.children(), id).is_some() {
            return Some((top.id, Part { child: Some(id), cell }));
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use deckcraft_model::table::{Cell, Row, Table};
    use deckcraft_model::text::TextBody;

    fn table(cells: &[&[&str]]) -> Shape {
        let rows = cells
            .iter()
            .map(|r| Row { height: 20.0, cells: r.iter().map(|t| Cell { text: TextBody::from_text(t), ..Default::default() }).collect() })
            .collect();
        Shape {
            id: ShapeId(7),
            kind: ShapeKind::Table(Table { cols: vec![50.0; cells.first().map_or(0, |r| r.len())], rows, ..Default::default() }),
            ..Default::default()
        }
    }

    #[test]
    fn text_is_found_in_cells_and_groups() {
        let t = table(&[&["a", "b"], &["c", "d"]]);
        let found: Vec<Part> = parts(&t).into_iter().map(|(p, _)| p).collect();
        assert_eq!(found.len(), 4);
        assert_eq!(found[3], Part { child: None, cell: Some((1, 1)) });
        let mut group = Shape {
            id: ShapeId(1),
            kind: ShapeKind::Group {
                children: vec![t, Shape { id: ShapeId(9), text: Some(TextBody::from_text("x")), ..Default::default() }],
                child: Default::default(),
            },
            ..Default::default()
        };
        let found: Vec<Part> = parts(&group).into_iter().map(|(p, _)| p).collect();
        assert!(found.contains(&Part { child: Some(ShapeId(7)), cell: Some((0, 1)) }));
        assert!(found.contains(&Part { child: Some(ShapeId(9)), cell: None }));
        let paras = part_mut(&mut group, &Part { child: Some(ShapeId(7)), cell: Some((1, 0)) }).unwrap();
        assert_eq!(paras[0].text(), "c");
        assert!(part_mut(&mut group, &Part { child: Some(ShapeId(7)), cell: Some((5, 0)) }).is_none());
        assert!(part_mut(&mut group, &Part { child: Some(ShapeId(99)), cell: None }).is_none());
        assert!(parts(&bare(&group)).iter().all(|(_, p)| p.is_empty()));
        assert_eq!(locate(std::slice::from_ref(&group), ShapeId(9), None), Some((ShapeId(1), Part { child: Some(ShapeId(9)), cell: None })));
    }
}
