use super::*;

const SQUARES: &[u8] = br##"<?xml version="1.0"?>
<!-- two halves -->
<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 20 10">
  <rect width="10" height="10" fill="#ff0000"/>
  <rect x="10" width="10" height="10" fill="#0000ff" fill-opacity="0.5"/>
</svg>"##;

fn px(r: &Raster, x: u32, y: u32) -> [u8; 4] {
    let i = ((y * r.width + x) * 4) as usize;
    [r.rgba[i], r.rgba[i + 1], r.rgba[i + 2], r.rgba[i + 3]]
}

#[test]
fn sniffs_svg_and_nothing_else() {
    assert!(is_svg(SQUARES));
    assert!(is_svg(b"\xEF\xBB\xBF  <svg xmlns='http://www.w3.org/2000/svg'/>"));
    assert!(!is_svg(b"\x89PNG\r\n\x1a\n"));
    assert!(!is_svg(b"<html><body>no</body></html>"));
    assert!(!is_svg(b"plain text mentioning <svg"));
    assert!(!is_svg(b""));
}

#[test]
fn natural_size_comes_from_the_view_box() {
    assert_eq!(natural_size(SQUARES), Some((20.0, 10.0)));
    let sized = br#"<svg xmlns="http://www.w3.org/2000/svg" width="96" height="48" viewBox="0 0 2 1"/>"#;
    assert_eq!(natural_size(sized), Some((96.0, 48.0)));
}

#[test]
fn draws_at_the_size_asked_with_straight_alpha() {
    let r = rasterize(SQUARES, 200).expect("draws");
    assert_eq!((r.width, r.height), (200, 100));
    assert_eq!(px(&r, 50, 50), [255, 0, 0, 255]);
    let blue = px(&r, 150, 50);
    assert_eq!((blue[0], blue[1], blue[2]), (0, 0, 255));
    assert!((126..=129).contains(&blue[3]), "half-transparent blue: {blue:?}");
}

#[test]
fn png_fallback_decodes_to_the_same_size() {
    let png = png(SQUARES, 64).expect("png");
    assert!(png.starts_with(b"\x89PNG\r\n\x1a\n"));
    // IHDR width and height.
    assert_eq!(u32::from_be_bytes([png[16], png[17], png[18], png[19]]), 64);
    assert_eq!(u32::from_be_bytes([png[20], png[21], png[22], png[23]]), 32);
}

#[test]
fn embedded_images_draw_but_files_are_never_read() {
    // A 1×1 opaque green PNG.
    let green =
        png(br##"<svg xmlns="http://www.w3.org/2000/svg" width="1" height="1"><rect width="1" height="1" fill="#00ff00"/></svg>"##, 1).expect("png");
    let b64 = base64(&green);
    let dir = std::env::temp_dir().join(format!("deckcraft-svg-test-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("dir");
    let file = dir.join("green.png");
    std::fs::write(&file, &green).expect("write");
    let path = file.to_string_lossy().replace('\\', "/");
    let svg = format!(
        r#"<svg xmlns="http://www.w3.org/2000/svg" xmlns:xlink="http://www.w3.org/1999/xlink" viewBox="0 0 2 1">
             <image width="1" height="1" xlink:href="data:image/png;base64,{b64}"/>
             <image x="1" width="1" height="1" xlink:href="{path}"/>
             <image x="1" width="1" height="1" xlink:href="file:///{path}"/>
           </svg>"#
    );
    let r = rasterize(svg.as_bytes(), 20).expect("draws");
    assert_eq!(px(&r, 5, 5), [0, 255, 0, 255], "the data: URL image draws");
    assert_eq!(px(&r, 15, 5)[3], 0, "the file image is not loaded");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn text_is_reported() {
    assert!(!has_text(SQUARES));
    assert!(has_text(br#"<svg xmlns="http://www.w3.org/2000/svg"><text>Hi</text></svg>"#));
    assert!(!has_text(b"<text>not an svg</text>"));
}

#[test]
fn hostile_input_is_none_never_a_panic() {
    for bad in [
        &b"<svg"[..],
        b"<svg xmlns='http://www.w3.org/2000/svg' width='0' height='0'/>",
        b"<svg xmlns='http://www.w3.org/2000/svg' width='-5' height='NaN'/>",
        b"<svg xmlns='http://www.w3.org/2000/svg'><rect width='1e999' height='1e999'/></svg>",
        b"\x00\x01\x02<svg>\xff\xfe",
    ] {
        let _ = natural_size(bad);
        let _ = rasterize(bad, 64);
        let _ = png(bad, 64);
    }
    // Huge declared size: drawn within the caps, not allocated at full size.
    let huge = b"<svg xmlns='http://www.w3.org/2000/svg' width='1000000' height='1000000'/>";
    let r = rasterize(huge, u32::MAX).expect("capped");
    assert!(r.width <= MAX_SIDE && r.height <= MAX_SIDE);
    // Too large to read at all.
    let mut big = b"<svg xmlns='http://www.w3.org/2000/svg'>".to_vec();
    big.resize(MAX_BYTES + 1, b' ');
    assert!(rasterize(&big, 16).is_none());
}

fn base64(data: &[u8]) -> String {
    const A: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut s = String::new();
    for c in data.chunks(3) {
        let n = (u32::from(c[0]) << 16) | (u32::from(*c.get(1).unwrap_or(&0)) << 8) | u32::from(*c.get(2).unwrap_or(&0));
        for i in 0..4 {
            if i <= c.len() {
                s.push(A[((n >> (18 - 6 * i)) & 63) as usize] as char);
            } else {
                s.push('=');
            }
        }
    }
    s
}
