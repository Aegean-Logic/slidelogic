//! SVG pictures (`asvg:svgBlip`): read with or without their raster copy, written back with one.

use std::collections::BTreeMap;
use std::io::{Cursor, Read, Write};

use deckcraft_geom::Xfrm;
use deckcraft_model::style::PictureFill;
use deckcraft_model::{Presentation, Shape, ShapeId, ShapeKind, defaults};

const SVG: &[u8] = br##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 40 20"><rect width="40" height="20" fill="#d4891c"/></svg>"##;
const OTHER_SVG: &[u8] = br##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 10 10"><circle cx="5" cy="5" r="5" fill="#4a6fb5"/></svg>"##;

fn parts(bytes: &[u8]) -> BTreeMap<String, Vec<u8>> {
    let mut z = zip::ZipArchive::new(Cursor::new(bytes)).expect("zip");
    (0..z.len())
        .map(|i| {
            let mut f = z.by_index(i).expect("entry");
            let mut d = vec![];
            f.read_to_end(&mut d).expect("read");
            (f.name().to_string(), d)
        })
        .collect()
}

fn rezip(parts: &BTreeMap<String, Vec<u8>>) -> Vec<u8> {
    let mut out = zip::ZipWriter::new(Cursor::new(Vec::new()));
    let o = zip::write::SimpleFileOptions::default();
    for (name, d) in parts {
        out.start_file(name.as_str(), o).expect("start");
        out.write_all(d).expect("write");
    }
    out.finish().expect("finish").into_inner()
}

fn slide1(parts: &BTreeMap<String, Vec<u8>>) -> String {
    String::from_utf8(parts["ppt/slides/slide1.xml"].clone()).expect("utf8")
}

/// The target of relationship `rid` in slide 1's relationships.
fn target(parts: &BTreeMap<String, Vec<u8>>, rid: &str) -> String {
    let rels = String::from_utf8(parts["ppt/slides/_rels/slide1.xml.rels"].clone()).expect("utf8");
    let at = rels.find(&format!("Id=\"{rid}\"")).expect("rel");
    let rel = &rels[rels[..at].rfind('<').expect("open")..];
    let t = &rel[rel.find("Target=\"").expect("target") + 8..];
    format!("ppt/{}", t[..t.find('"').expect("end")].trim_start_matches("../"))
}

/// `r:embed` of `<tag` in `xml`.
fn embed(xml: &str, tag: &str) -> String {
    let e = &xml[xml.find(tag).unwrap_or_else(|| panic!("{tag} in {xml}"))..];
    let e = &e[..e.find('>').expect("end")];
    let v = &e[e.find("r:embed=\"").unwrap_or_else(|| panic!("r:embed in {e}")) + 9..];
    v[..v.find('"').expect("quote")].to_string()
}

/// A deck with one picture showing `svg` (as Insert ▸ Pictures adds it: no raster copy).
fn deck(svg: &[u8]) -> Presentation {
    let mut p = defaults::new_presentation(None);
    let media = p.add_media("logo.svg", "image/svg+xml", svg.to_vec()).expect("media");
    let id = ShapeId(p.alloc_id().expect("ids"));
    let first = p.slides[0].id;
    p.slide_mut(first).expect("slide").shapes.push(Shape {
        id,
        name: "Logo".into(),
        xfrm: Some(Xfrm::new(10.0, 10.0, 200.0, 100.0)),
        kind: ShapeKind::Picture { fill: PictureFill { media, ..Default::default() } },
        ..Default::default()
    });
    p
}

fn logo(p: &Presentation) -> &PictureFill {
    let s = p.slides[0].shapes.iter().find(|s| s.name == "Logo").expect("logo");
    match &s.kind {
        ShapeKind::Picture { fill } => fill,
        other => panic!("{other:?}"),
    }
}

fn data(p: &Presentation, id: deckcraft_model::MediaId) -> Vec<u8> {
    p.media(id).expect("media").data.as_ref().clone()
}

#[test]
fn an_svg_picture_is_saved_with_a_png_copy_and_reads_back_as_svg() {
    let bytes = deckcraft_pptx::export(&deck(SVG)).expect("export");
    let parts = parts(&bytes);
    let xml = slide1(&parts);
    let png = &parts[&target(&parts, &embed(&xml, "<a:blip "))];
    assert!(png.starts_with(b"\x89PNG\r\n\x1a\n"), "a:blip names a PNG");
    assert_eq!(parts[&target(&parts, &embed(&xml, "<asvg:svgBlip"))], SVG, "the SVG rides in asvg:svgBlip");
    assert!(xml.contains("{96DAC541-7B7A-43D3-8B79-37D633B846F1}"));

    let q = deckcraft_pptx::import(&bytes).expect("import");
    let pf = logo(&q);
    assert_eq!(data(&q, pf.media), SVG, "the picture is the SVG");
    let fb = pf.svg_fallback.expect("its PNG copy is kept");
    assert_eq!(fb.svg, pf.media);
    assert_eq!(&data(&q, fb.raster), png);
}

#[test]
fn an_svg_without_a_raster_copy_is_kept_and_gains_one() {
    // As some tools write it (and the file behind this fix): `<a:blip>` with only the SVG.
    let mut parts = parts(&deckcraft_pptx::export(&deck(SVG)).expect("export"));
    let xml = slide1(&parts);
    let rid = embed(&xml, "<a:blip ");
    let stripped = xml.replace(&format!("<a:blip r:embed=\"{rid}\">"), "<a:blip>");
    assert_ne!(stripped, xml);
    parts.insert("ppt/slides/slide1.xml".into(), stripped.into_bytes());
    let q = deckcraft_pptx::import(&rezip(&parts)).expect("import");
    let pf = logo(&q);
    assert_eq!(data(&q, pf.media), SVG, "the SVG alone is the picture");
    assert_eq!(pf.svg_fallback, None);

    let again = self::parts(&deckcraft_pptx::export(&q).expect("export"));
    let xml = slide1(&again);
    assert!(again[&target(&again, &embed(&xml, "<a:blip "))].starts_with(b"\x89PNG"));
    assert_eq!(again[&target(&again, &embed(&xml, "<asvg:svgBlip"))], SVG);
}

#[test]
fn the_raster_copy_read_is_written_back_unchanged() {
    let first = deckcraft_pptx::import(&deckcraft_pptx::export(&deck(SVG)).expect("export")).expect("import");
    let raster = data(&first, logo(&first).svg_fallback.expect("copy").raster);
    let second = deckcraft_pptx::import(&deckcraft_pptx::export(&first).expect("export")).expect("import");
    assert_eq!(data(&second, logo(&second).svg_fallback.expect("copy").raster), raster);
}

#[test]
fn a_changed_picture_drops_the_old_svgs_raster_copy() {
    let mut p = deckcraft_pptx::import(&deckcraft_pptx::export(&deck(SVG)).expect("export")).expect("import");
    let old = logo(&p).svg_fallback.expect("copy");
    let other = p.add_media("other.svg", "image/svg+xml", OTHER_SVG.to_vec()).expect("media");
    let first = p.slides[0].id;
    for s in &mut p.slide_mut(first).expect("slide").shapes {
        if let ShapeKind::Picture { fill } = &mut s.kind {
            fill.media = other;
        }
    }
    assert_eq!(logo(&p).svg_fallback, Some(old), "the stale copy is still recorded");
    assert_eq!(logo(&p).svg_fallback_media(), None, "but no longer applies");
    let parts = parts(&deckcraft_pptx::export(&p).expect("export"));
    let xml = slide1(&parts);
    assert_eq!(parts[&target(&parts, &embed(&xml, "<asvg:svgBlip"))], OTHER_SVG);
    assert_ne!(parts[&target(&parts, &embed(&xml, "<a:blip "))], data(&p, old.raster), "a fresh copy of the new SVG");
}

#[test]
fn a_raster_picture_has_no_svg_extension() {
    let mut p = defaults::new_presentation(None);
    let media = p.add_media("dot.png", "image/png", deckcraft_svg::png(OTHER_SVG, 8).expect("png")).expect("media");
    let id = ShapeId(p.alloc_id().expect("ids"));
    let first = p.slides[0].id;
    p.slide_mut(first).expect("slide").shapes.push(Shape {
        id,
        name: "Logo".into(),
        xfrm: Some(Xfrm::new(10.0, 10.0, 20.0, 20.0)),
        kind: ShapeKind::Picture { fill: PictureFill { media, ..Default::default() } },
        ..Default::default()
    });
    let xml = slide1(&parts(&deckcraft_pptx::export(&p).expect("export")));
    assert!(!xml.contains("svgBlip"));
    assert!(xml.contains("<a:blip r:embed="));
}
