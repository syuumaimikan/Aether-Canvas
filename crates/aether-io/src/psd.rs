//! Photoshop documents (`.psd`).
//!
//! Layered PSD is how most rigging work starts: artwork is painted elsewhere,
//! split into parts, and handed to the rigger. Importing keeps that structure:
//! every pixel layer becomes a raster layer with its name (including Unicode
//! names), opacity, visibility, blend mode, clipping and layer mask, and
//! folders become groups (pass-through or isolated, as in the file).
//!
//! Exporting writes the same structure back, so a rigged document can go
//! round-trip to a painting application for touch-ups.
//!
//! Supported: version 1 files, 8 bits per channel, RGB or grayscale, raw or
//! RLE (PackBits) channel data. Other variants are refused with a message
//! saying why, rather than producing wrong pixels.

use aether_core::blend::BlendMode;
use aether_core::color::Rgba8;
use aether_core::math::IRect;
use aether_core::{AetherError, LayerId, Result};
use aether_document::layer::{GroupContent, Layer, LayerContent};
use aether_document::Document;
use aether_raster::{Mask, Pixmap};

fn bad(message: impl std::fmt::Display) -> AetherError {
    AetherError::UnsupportedFormat(format!("PSD: {message}"))
}

/// A bounds-checked big-endian reader.
struct Reader<'a> {
    data: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    fn new(data: &'a [u8]) -> Self {
        Self { data, pos: 0 }
    }

    fn take(&mut self, n: usize) -> Result<&'a [u8]> {
        let end = self
            .pos
            .checked_add(n)
            .filter(|e| *e <= self.data.len())
            .ok_or_else(|| bad("the file is truncated"))?;
        let slice = &self.data[self.pos..end];
        self.pos = end;
        Ok(slice)
    }

    fn u8(&mut self) -> Result<u8> {
        Ok(self.take(1)?[0])
    }

    fn u16(&mut self) -> Result<u16> {
        let b = self.take(2)?;
        Ok(u16::from_be_bytes([b[0], b[1]]))
    }

    fn i16(&mut self) -> Result<i16> {
        Ok(self.u16()? as i16)
    }

    fn u32(&mut self) -> Result<u32> {
        let b = self.take(4)?;
        Ok(u32::from_be_bytes([b[0], b[1], b[2], b[3]]))
    }

    fn i32(&mut self) -> Result<i32> {
        Ok(self.u32()? as i32)
    }

    fn skip(&mut self, n: usize) -> Result<()> {
        self.take(n).map(|_| ())
    }
}

/// One channel of one layer record.
struct ChannelInfo {
    id: i16,
    length: usize,
}

/// A parsed layer record, before its pixels are read.
struct Record {
    rect: IRect,
    channels: Vec<ChannelInfo>,
    blend: [u8; 4],
    opacity: u8,
    clipping: bool,
    hidden: bool,
    name: String,
    /// 0 = normal layer, 1/2 = open/closed folder, 3 = folder end marker.
    section: u32,
    mask: Option<(IRect, u8, bool)>,
}

/// Map a PSD blend key to a blend mode (`None` means pass-through).
fn blend_from_key(key: &[u8; 4]) -> Option<BlendMode> {
    Some(match key {
        b"norm" | b"diss" => BlendMode::Normal,
        b"mul " => BlendMode::Multiply,
        b"scrn" => BlendMode::Screen,
        b"over" => BlendMode::Overlay,
        b"dark" | b"dkCl" => BlendMode::Darken,
        b"lite" | b"lgCl" => BlendMode::Lighten,
        b"div " => BlendMode::ColorDodge,
        b"idiv" => BlendMode::ColorBurn,
        b"hLit" => BlendMode::HardLight,
        b"sLit" => BlendMode::SoftLight,
        b"diff" => BlendMode::Difference,
        b"smud" => BlendMode::Exclusion,
        b"lddg" => BlendMode::Add,
        b"fsub" => BlendMode::Subtract,
        b"fdiv" => BlendMode::Divide,
        b"lbrn" => BlendMode::LinearBurn,
        b"vLit" => BlendMode::VividLight,
        b"lLit" => BlendMode::LinearLight,
        b"pLit" => BlendMode::PinLight,
        b"hue " => BlendMode::Hue,
        b"sat " => BlendMode::Saturation,
        b"colr" => BlendMode::Color,
        b"lum " => BlendMode::Luminosity,
        b"pass" => return None,
        _ => BlendMode::Normal,
    })
}

fn key_from_blend(mode: BlendMode) -> &'static [u8; 4] {
    match mode {
        BlendMode::Normal => b"norm",
        BlendMode::Multiply => b"mul ",
        BlendMode::Screen => b"scrn",
        BlendMode::Overlay => b"over",
        BlendMode::Darken => b"dark",
        BlendMode::Lighten => b"lite",
        BlendMode::ColorDodge => b"div ",
        BlendMode::ColorBurn => b"idiv",
        BlendMode::HardLight => b"hLit",
        BlendMode::SoftLight => b"sLit",
        BlendMode::Difference => b"diff",
        BlendMode::Exclusion => b"smud",
        BlendMode::Add => b"lddg",
        BlendMode::Subtract => b"fsub",
        BlendMode::Divide => b"fdiv",
        BlendMode::LinearBurn => b"lbrn",
        BlendMode::VividLight => b"vLit",
        BlendMode::LinearLight => b"lLit",
        BlendMode::PinLight => b"pLit",
        BlendMode::Hue => b"hue ",
        BlendMode::Saturation => b"sat ",
        BlendMode::Color => b"colr",
        BlendMode::Luminosity => b"lum ",
    }
}

/// Decode PackBits-compressed rows.
fn unpack_bits(input: &[u8], expected: usize) -> Result<Vec<u8>> {
    let mut out = Vec::with_capacity(expected);
    let mut i = 0;
    while i < input.len() && out.len() < expected {
        let n = input[i] as i8;
        i += 1;
        if n >= 0 {
            let count = n as usize + 1;
            let end = (i + count).min(input.len());
            out.extend_from_slice(&input[i..end]);
            i = end;
        } else if n != -128 {
            let count = (-(n as isize)) as usize + 1;
            let value = *input.get(i).ok_or_else(|| bad("an RLE run is truncated"))?;
            i += 1;
            out.extend(std::iter::repeat_n(value, count));
        }
    }
    out.resize(expected, 0);
    Ok(out)
}

/// Read one channel's pixels (`width × height` bytes) from its data block.
fn read_channel(data: &[u8], width: usize, height: usize) -> Result<Vec<u8>> {
    let mut r = Reader::new(data);
    let compression = r.u16()?;
    let size = width * height;
    match compression {
        0 => {
            let raw = r.take(size.min(data.len().saturating_sub(2)))?;
            let mut out = raw.to_vec();
            out.resize(size, 0);
            Ok(out)
        }
        1 => {
            let mut counts = Vec::with_capacity(height);
            for _ in 0..height {
                counts.push(r.u16()? as usize);
            }
            let mut out = Vec::with_capacity(size);
            for count in counts {
                let row = r.take(count)?;
                out.extend(unpack_bits(row, width)?);
            }
            Ok(out)
        }
        2 | 3 => Err(bad(
            "ZIP-compressed channels are not supported; re-save without compression",
        )),
        other => Err(bad(format!("unknown channel compression {other}"))),
    }
}

/// Parse a PSD file into a document.
pub fn load_psd(bytes: &[u8]) -> Result<Document> {
    let mut r = Reader::new(bytes);
    if r.take(4)? != b"8BPS" {
        return Err(bad("not a Photoshop document"));
    }
    match r.u16()? {
        1 => {}
        2 => return Err(bad("large-document (PSB) files are not supported")),
        v => return Err(bad(format!("unknown version {v}"))),
    }
    r.skip(6)?;
    let channel_count = r.u16()? as usize;
    let height = r.u32()?;
    let width = r.u32()?;
    let depth = r.u16()?;
    let mode = r.u16()?;
    if depth != 8 {
        return Err(bad(format!(
            "{depth}-bit channels are not supported (8-bit only)"
        )));
    }
    if mode != 3 && mode != 1 {
        return Err(bad("only RGB and grayscale documents are supported"));
    }
    if width == 0 || height == 0 || width > 30_000 || height > 30_000 {
        return Err(bad("the canvas size is out of range"));
    }
    let gray = mode == 1;

    // Colour mode data and image resources are not needed.
    let len = r.u32()? as usize;
    r.skip(len)?;
    let len = r.u32()? as usize;
    r.skip(len)?;

    let mut doc = Document::empty(width, height, "Imported PSD");
    let layer_section_len = r.u32()? as usize;
    let section_start = r.pos;
    let mut records: Vec<Record> = Vec::new();
    let mut pixels: Vec<(Pixmap, Option<Mask>)> = Vec::new();
    if layer_section_len > 0 {
        let info_len = r.u32()? as usize;
        if info_len > 0 {
            let count = r.i16()?.unsigned_abs() as usize;
            for _ in 0..count {
                records.push(read_record(&mut r)?);
            }
            for record in &records {
                pixels.push(read_layer_pixels(&mut r, record, width, height, gray)?);
            }
        }
        r.pos = section_start + layer_section_len;
    }

    if records.is_empty() {
        // A flat file: use the merged image as a single layer.
        let pixmap = read_merged(&mut r, width, height, channel_count, gray)?;
        let id = doc.next_layer_id();
        let mut layer = Layer::raster(id, "Background", width, height);
        if let Some(target) = layer.pixmap_mut() {
            *target = pixmap;
        }
        doc.layers.push_top(layer)?;
        doc.active_layer = id;
        doc.repair();
        return Ok(doc);
    }

    // Records run bottom to top; folders are bracketed by an end marker
    // (below their children) and the folder record itself (above them).
    let mut stack: Vec<Vec<Node>> = vec![Vec::new()];
    for (record, (pixmap, mask)) in records.into_iter().zip(pixels) {
        match record.section {
            3 => stack.push(Vec::new()),
            1 | 2 => {
                let children = if stack.len() > 1 {
                    stack.pop().unwrap_or_default()
                } else {
                    Vec::new()
                };
                let id = doc.next_layer_id();
                let blend = blend_from_key(&record.blend);
                let mut group = Layer::with_content(
                    id,
                    record.name,
                    LayerContent::Group(GroupContent {
                        children: Vec::new(),
                        isolate: blend.is_some(),
                        collapsed: record.section == 2,
                    }),
                );
                group.blend_mode = blend.unwrap_or(BlendMode::Normal);
                group.opacity = record.opacity as f32 / 255.0;
                group.visible = !record.hidden;
                group.mask = mask;
                stack
                    .last_mut()
                    .ok_or_else(|| bad("folder structure is broken"))?
                    .push(Node::Folder(group, children));
            }
            _ => {
                let id = doc.next_layer_id();
                let mut layer = Layer::raster(id, record.name, width, height);
                if let Some(target) = layer.pixmap_mut() {
                    *target = pixmap;
                }
                layer.blend_mode = blend_from_key(&record.blend).unwrap_or(BlendMode::Normal);
                layer.opacity = record.opacity as f32 / 255.0;
                layer.visible = !record.hidden;
                layer.clipping = record.clipping;
                layer.mask = mask;
                stack
                    .last_mut()
                    .ok_or_else(|| bad("folder structure is broken"))?
                    .push(Node::Layer(layer));
            }
        }
    }
    // Unterminated folders: fold their contents back into the parent.
    while stack.len() > 1 {
        let orphans = stack.pop().unwrap_or_default();
        if let Some(parent) = stack.last_mut() {
            parent.extend(orphans);
        }
    }
    let roots = stack.pop().unwrap_or_default();
    insert_layers(&mut doc, roots, None)?;
    doc.active_layer = doc
        .layers
        .iter_ui_order()
        .into_iter()
        .map(|(id, _)| id)
        .find(|id| doc.layers.get(*id).is_some_and(|l| !l.is_group()))
        .unwrap_or(LayerId::NONE);
    doc.repair();
    Ok(doc)
}

/// A layer as read from the file, with folder contents attached.
enum Node {
    Layer(Layer),
    Folder(Layer, Vec<Node>),
}

/// Insert nodes bottom-to-top under `parent`, recursing into folders.
fn insert_layers(doc: &mut Document, nodes: Vec<Node>, parent: Option<LayerId>) -> Result<()> {
    for node in nodes {
        let index = doc.layers.children_of(parent).len();
        match node {
            Node::Layer(layer) => {
                doc.layers.insert(layer, parent, index)?;
            }
            Node::Folder(group, children) => {
                let id = group.id;
                doc.layers.insert(group, parent, index)?;
                insert_layers(doc, children, Some(id))?;
            }
        }
    }
    Ok(())
}

fn read_record(r: &mut Reader) -> Result<Record> {
    let top = r.i32()?;
    let left = r.i32()?;
    let bottom = r.i32()?;
    let right = r.i32()?;
    let channel_count = r.u16()? as usize;
    if channel_count > 56 {
        return Err(bad("a layer has an impossible number of channels"));
    }
    let mut channels = Vec::with_capacity(channel_count);
    for _ in 0..channel_count {
        let id = r.i16()?;
        let length = r.u32()? as usize;
        channels.push(ChannelInfo { id, length });
    }
    if r.take(4)? != b"8BIM" {
        return Err(bad("a layer record is malformed"));
    }
    let mut blend = [0u8; 4];
    blend.copy_from_slice(r.take(4)?);
    let opacity = r.u8()?;
    let clipping = r.u8()? != 0;
    let flags = r.u8()?;
    let _filler = r.u8()?;
    let extra_len = r.u32()? as usize;
    let extra = r.take(extra_len)?;
    let mut e = Reader::new(extra);

    // Layer mask.
    let mask_len = e.u32()? as usize;
    let mask_data = e.take(mask_len)?;
    let mask = if mask_len >= 18 {
        let mut m = Reader::new(mask_data);
        let (mt, ml, mb, mr) = (m.i32()?, m.i32()?, m.i32()?, m.i32()?);
        let default = m.u8()?;
        let mask_flags = m.u8()?;
        Some((
            IRect::from_bounds(ml, mt, mr, mb),
            default,
            mask_flags & 0x02 != 0,
        ))
    } else {
        None
    };
    // Blending ranges.
    let ranges = e.u32()? as usize;
    e.skip(ranges)?;
    // Pascal name, padded to a multiple of four.
    let name_len = e.u8()? as usize;
    let raw_name = e.take(name_len)?;
    let padded = (1 + name_len).div_ceil(4) * 4;
    e.skip(padded - 1 - name_len)?;
    let mut name: String = raw_name.iter().map(|&b| b as char).collect();
    let mut section = 0;
    // Additional layer information.
    while e.pos + 12 <= extra.len() {
        let signature = e.take(4)?;
        if signature != b"8BIM" && signature != b"8B64" {
            break;
        }
        let key = e.take(4)?;
        let len = e.u32()? as usize;
        let data = e.take(len.min(extra.len() - e.pos))?;
        match key {
            b"luni" => {
                let mut d = Reader::new(data);
                let count = d.u32()? as usize;
                let mut units = Vec::with_capacity(count);
                for _ in 0..count {
                    units.push(d.u16()?);
                }
                if let Ok(unicode) = String::from_utf16(&units) {
                    name = unicode.trim_end_matches('\0').to_string();
                }
            }
            b"lsct" | b"lsdk" => {
                let mut d = Reader::new(data);
                section = d.u32()?;
                if data.len() >= 12 {
                    let _sig = d.take(4)?;
                    blend.copy_from_slice(d.take(4)?);
                }
            }
            _ => {}
        }
        // Blocks are padded to an even length.
        if len % 2 == 1 && e.pos < extra.len() {
            e.skip(1)?;
        }
    }
    Ok(Record {
        rect: IRect::from_bounds(left, top, right, bottom),
        channels,
        blend,
        opacity,
        clipping,
        hidden: flags & 0x02 != 0,
        name,
        section,
        mask,
    })
}

fn read_layer_pixels(
    r: &mut Reader,
    record: &Record,
    width: u32,
    height: u32,
    gray: bool,
) -> Result<(Pixmap, Option<Mask>)> {
    let rect = record.rect;
    let (w, h) = (rect.width.max(0) as usize, rect.height.max(0) as usize);
    let mut planes: [Option<Vec<u8>>; 4] = [None, None, None, None];
    let mut mask_plane: Option<Vec<u8>> = None;
    for channel in &record.channels {
        let data = r.take(channel.length)?;
        match channel.id {
            0..=2 if w > 0 && h > 0 => planes[channel.id as usize] = Some(read_channel(data, w, h)?),
            -1 if w > 0 && h > 0 => planes[3] = Some(read_channel(data, w, h)?),
            -2 => {
                if let Some((mrect, _, _)) = record.mask {
                    let (mw, mh) = (mrect.width.max(0) as usize, mrect.height.max(0) as usize);
                    if mw > 0 && mh > 0 {
                        mask_plane = Some(read_channel(data, mw, mh)?);
                    }
                }
            }
            _ => {}
        }
    }
    let mut pixmap = Pixmap::new(width, height);
    if w > 0 && h > 0 {
        let channel = |i: usize, index: usize| planes[i].as_ref().map(|p| p[index]);
        for y in 0..h {
            for x in 0..w {
                let (px, py) = (rect.x + x as i32, rect.y + y as i32);
                if px < 0 || py < 0 || px >= width as i32 || py >= height as i32 {
                    continue;
                }
                let i = y * w + x;
                let r_ = channel(0, i).unwrap_or(0);
                let (g, b) = if gray {
                    (r_, r_)
                } else {
                    (channel(1, i).unwrap_or(0), channel(2, i).unwrap_or(0))
                };
                let a = channel(3, i).unwrap_or(255);
                pixmap.set(px, py, Rgba8::new(r_, g, b, a));
            }
        }
    }
    let mask = match (record.mask, mask_plane) {
        (Some((mrect, default, disabled)), plane) if !disabled => {
            let mut mask = Mask::filled(width, height, default);
            if let Some(plane) = plane {
                let mw = mrect.width.max(0) as usize;
                for (i, value) in plane.iter().enumerate() {
                    let (x, y) = (mrect.x + (i % mw) as i32, mrect.y + (i / mw) as i32);
                    mask.set(x, y, *value);
                }
            }
            Some(mask)
        }
        _ => None,
    };
    Ok((pixmap, mask))
}

fn read_merged(r: &mut Reader, width: u32, height: u32, channels: usize, gray: bool) -> Result<Pixmap> {
    let compression = r.u16()?;
    let (w, h) = (width as usize, height as usize);
    // Colour planes plus an optional alpha plane, as the header declares.
    let plane_count = channels.clamp(1, if gray { 2 } else { 4 });
    let mut planes: Vec<Vec<u8>> = Vec::new();
    match compression {
        0 => {
            for _ in 0..plane_count {
                planes.push(r.take(w * h)?.to_vec());
            }
        }
        1 => {
            // Byte counts for every row of every plane come first.
            let mut counts = Vec::with_capacity(h * plane_count);
            for _ in 0..h * plane_count {
                counts.push(r.u16()? as usize);
            }
            for c in 0..plane_count {
                let mut plane = Vec::with_capacity(w * h);
                for row in 0..h {
                    let data = r.take(counts[c * h + row])?;
                    plane.extend(unpack_bits(data, w)?);
                }
                planes.push(plane);
            }
        }
        _ => return Err(bad("the merged image uses an unsupported compression")),
    }
    let mut pixmap = Pixmap::new(width, height);
    for y in 0..h {
        for x in 0..w {
            let i = y * w + x;
            let v = |c: usize| planes.get(c).map(|p| p[i]);
            let red = v(0).unwrap_or(0);
            let (g, b, a) = if gray {
                (red, red, v(1).unwrap_or(255))
            } else {
                (v(1).unwrap_or(0), v(2).unwrap_or(0), v(3).unwrap_or(255))
            };
            pixmap.set(x as i32, y as i32, Rgba8::new(red, g, b, a));
        }
    }
    Ok(pixmap)
}

/// Read a PSD from disk.
pub fn load_psd_file(path: impl AsRef<std::path::Path>) -> Result<Document> {
    let bytes = std::fs::read(path.as_ref())?;
    let mut doc = load_psd(&bytes)?;
    if let Some(stem) = path.as_ref().file_stem() {
        doc.name = stem.to_string_lossy().to_string();
    }
    Ok(doc)
}

// ------------------------------------------------------------------ writing

struct Writer {
    out: Vec<u8>,
}

impl Writer {
    fn u8(&mut self, v: u8) {
        self.out.push(v);
    }
    fn u16(&mut self, v: u16) {
        self.out.extend_from_slice(&v.to_be_bytes());
    }
    fn u32(&mut self, v: u32) {
        self.out.extend_from_slice(&v.to_be_bytes());
    }
    fn i32(&mut self, v: i32) {
        self.out.extend_from_slice(&v.to_be_bytes());
    }
    fn bytes(&mut self, b: &[u8]) {
        self.out.extend_from_slice(b);
    }
}

/// A flattened layer record for writing: what to write and its pixels.
struct OutLayer {
    name: String,
    rect: IRect,
    blend: &'static [u8; 4],
    opacity: u8,
    clipping: bool,
    hidden: bool,
    section: u32,
    pixels: Option<Pixmap>,
}

fn collect(doc: &Document, parent: Option<LayerId>, out: &mut Vec<OutLayer>) {
    for id in doc.layers.children_of(parent) {
        let Some(layer) = doc.layers.get(*id) else {
            continue;
        };
        let opacity = (layer.opacity.clamp(0.0, 1.0) * 255.0).round() as u8;
        match &layer.content {
            LayerContent::Group(g) => {
                out.push(OutLayer {
                    name: "</Layer group>".into(),
                    rect: IRect::EMPTY,
                    blend: b"norm",
                    opacity: 255,
                    clipping: false,
                    hidden: false,
                    section: 3,
                    pixels: None,
                });
                collect(doc, Some(*id), out);
                out.push(OutLayer {
                    name: layer.name.clone(),
                    rect: IRect::EMPTY,
                    blend: if g.isolate {
                        key_from_blend(layer.blend_mode)
                    } else {
                        b"pass"
                    },
                    opacity,
                    clipping: false,
                    hidden: !layer.visible,
                    section: if g.collapsed { 2 } else { 1 },
                    pixels: None,
                });
            }
            LayerContent::Raster(raster) => {
                let rect = raster.pixmap.opaque_bounds();
                out.push(OutLayer {
                    name: layer.name.clone(),
                    rect,
                    blend: key_from_blend(layer.blend_mode),
                    opacity,
                    clipping: layer.clipping,
                    hidden: !layer.visible,
                    section: 0,
                    pixels: (!rect.is_empty()).then(|| raster.pixmap.copy_rect(rect)),
                });
            }
            // Adjustment, fill and plugin layers have no PSD equivalent here;
            // they are represented by the merged image only.
            _ => {}
        }
    }
}

/// Encode a document as a layered PSD (raw channel data, RGB + alpha).
///
/// The merged image is the full composite, so applications that ignore
/// layers still show the right picture.
pub fn save_psd(doc: &Document, composite: &Pixmap) -> Result<Vec<u8>> {
    let mut layers = Vec::new();
    collect(doc, None, &mut layers);
    let mut w = Writer { out: Vec::new() };
    w.bytes(b"8BPS");
    w.u16(1);
    w.bytes(&[0; 6]);
    w.u16(4);
    w.u32(doc.height);
    w.u32(doc.width);
    w.u16(8);
    w.u16(3);
    w.u32(0); // colour mode data
    w.u32(0); // image resources

    // Layer info, assembled separately to know its length.
    let mut info = Writer { out: Vec::new() };
    info.u16(layers.len() as u16);
    let channel_bytes = |l: &OutLayer| (l.rect.width.max(0) * l.rect.height.max(0)) as u32 + 2;
    for layer in &layers {
        info.i32(layer.rect.y);
        info.i32(layer.rect.x);
        info.i32(layer.rect.bottom());
        info.i32(layer.rect.right());
        info.u16(4);
        for id in [-1i16, 0, 1, 2] {
            info.u16(id as u16);
            info.u32(channel_bytes(layer));
        }
        info.bytes(b"8BIM");
        info.bytes(layer.blend);
        info.u8(layer.opacity);
        info.u8(layer.clipping as u8);
        info.u8(if layer.hidden { 0x02 } else { 0 });
        info.u8(0);
        let mut extra = Writer { out: Vec::new() };
        extra.u32(0); // no mask
        extra.u32(0); // no blending ranges
        let ascii: Vec<u8> = layer
            .name
            .chars()
            .map(|c| if c.is_ascii() { c as u8 } else { b'?' })
            .take(255)
            .collect();
        extra.u8(ascii.len() as u8);
        extra.bytes(&ascii);
        let padded = (1 + ascii.len()).div_ceil(4) * 4;
        extra.bytes(&vec![0; padded - 1 - ascii.len()]);
        // Unicode name.
        let units: Vec<u16> = layer.name.encode_utf16().collect();
        extra.bytes(b"8BIMluni");
        let len = 4 + units.len() * 2;
        extra.u32(len as u32);
        extra.u32(units.len() as u32);
        for u in &units {
            extra.u16(*u);
        }
        if len % 2 == 1 {
            extra.u8(0);
        }
        if layer.section != 0 {
            extra.bytes(b"8BIMlsct");
            extra.u32(12);
            extra.u32(layer.section);
            extra.bytes(b"8BIM");
            extra.bytes(layer.blend);
        }
        info.u32(extra.out.len() as u32);
        info.bytes(&extra.out);
    }
    for layer in &layers {
        let (lw, lh) = (
            layer.rect.width.max(0) as usize,
            layer.rect.height.max(0) as usize,
        );
        for channel in [3usize, 0, 1, 2] {
            info.u16(0);
            if let Some(p) = &layer.pixels {
                let data = p.data();
                for i in 0..lw * lh {
                    info.u8(data[i * 4 + channel]);
                }
            }
        }
    }
    if info.out.len() % 2 == 1 {
        info.u8(0);
    }
    w.u32(info.out.len() as u32 + 8);
    w.u32(info.out.len() as u32);
    w.bytes(&info.out);
    w.u32(0); // global layer mask info

    // Merged image, planar, raw.
    w.u16(0);
    let data = composite.data();
    for channel in 0..4 {
        for i in 0..(doc.width * doc.height) as usize {
            w.u8(data[i * 4 + channel]);
        }
    }
    Ok(w.out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use aether_render::Compositor;

    fn sample() -> Document {
        let mut doc = Document::empty(32, 24, "psd");
        let base = doc.next_layer_id();
        let mut layer = Layer::raster(base, "Base", 32, 24);
        if let Some(pm) = layer.pixmap_mut() {
            pm.fill_rect(IRect::new(2, 2, 20, 16), Rgba8::rgb(200, 30, 40));
        }
        doc.layers.push_top(layer).expect("layer");
        let group = doc.next_layer_id();
        doc.layers
            .push_top(Layer::group(group, "顔パーツ"))
            .expect("group");
        let eye = doc.next_layer_id();
        let mut eye_layer = Layer::raster(eye, "目", 32, 24);
        if let Some(pm) = eye_layer.pixmap_mut() {
            pm.fill_rect(IRect::new(6, 6, 4, 3), Rgba8::new(10, 20, 250, 200));
        }
        eye_layer.blend_mode = BlendMode::Multiply;
        eye_layer.opacity = 0.5;
        doc.layers.insert(eye_layer, Some(group), 0).expect("child");
        let clip = doc.next_layer_id();
        let mut clip_layer = Layer::raster(clip, "Shade", 32, 24);
        if let Some(pm) = clip_layer.pixmap_mut() {
            pm.fill_rect(IRect::new(0, 0, 32, 24), Rgba8::rgb(0, 0, 0));
        }
        clip_layer.clipping = true;
        clip_layer.visible = false;
        doc.layers.insert(clip_layer, Some(group), 1).expect("clip");
        doc.active_layer = base;
        doc
    }

    #[test]
    fn a_layered_document_round_trips_through_psd() {
        let doc = sample();
        let composite = Compositor::new().render(&doc);
        let bytes = save_psd(&doc, &composite).expect("save");
        let back = load_psd(&bytes).expect("load");
        let names: Vec<String> = back
            .layers
            .iter_ui_order()
            .into_iter()
            .filter_map(|(id, _)| back.layers.get(id).map(|l| l.name.clone()))
            .collect();
        assert_eq!(
            names,
            vec!["顔パーツ", "Shade", "目", "Base"],
            "top-to-bottom order with Unicode names"
        );
        let eye = back.layers.iter().find(|l| l.name == "目").expect("eye layer");
        assert_eq!(eye.blend_mode, BlendMode::Multiply);
        assert!((eye.opacity - 0.5).abs() < 0.01);
        assert_eq!(
            eye.pixmap().map(|p| p.get(7, 7)),
            Some(Rgba8::new(10, 20, 250, 200))
        );
        let group_id = back.layers.parent_of(eye.id).expect("inside the folder");
        assert!(back.layers.get(group_id).is_some_and(|g| g.is_group()));
        let shade = back.layers.iter().find(|l| l.name == "Shade").expect("shade");
        assert!(shade.clipping && !shade.visible);
        // PSD stores opacity in 8 bits (0.5 becomes 128/255), so allow one
        // level of difference.
        let again = Compositor::new().render(&back);
        let worst = again
            .data()
            .iter()
            .zip(composite.data())
            .map(|(a, b)| (*a as i32 - *b as i32).abs())
            .max()
            .unwrap_or(0);
        assert!(worst <= 1, "the picture changed by up to {worst} levels");
    }

    #[test]
    fn rle_channels_decode() {
        assert_eq!(
            unpack_bits(&[2, 1, 2, 3, 0xFD, 9], 7).expect("rle"),
            vec![1, 2, 3, 9, 9, 9, 9]
        );
        // Truncated runs are an error, not a panic.
        assert!(unpack_bits(&[0xFD], 4).is_err());
    }

    #[test]
    fn unsupported_files_explain_themselves() {
        let err = load_psd(b"GIF89a").expect_err("not psd");
        assert!(err.to_string().contains("not a Photoshop"), "{err}");
        let mut header = b"8BPS".to_vec();
        header.extend_from_slice(&2u16.to_be_bytes());
        header.extend_from_slice(&[0; 20]);
        let err = load_psd(&header).expect_err("psb");
        assert!(err.to_string().contains("PSB"), "{err}");
        let err = load_psd(&header[..10]).expect_err("truncated");
        assert!(err.to_string().contains("PSD"), "{err}");
    }

    #[test]
    fn a_flat_psd_becomes_one_layer() {
        let mut doc = Document::empty(8, 8, "flat");
        let id = doc.next_layer_id();
        let mut layer = Layer::raster(id, "x", 8, 8);
        if let Some(pm) = layer.pixmap_mut() {
            pm.fill(Rgba8::rgb(1, 2, 3));
        }
        doc.layers.push_top(layer).expect("layer");
        let composite = Compositor::new().render(&doc);
        let mut empty = doc.clone();
        empty.layers = aether_document::LayerTree::new();
        let bytes = save_psd(&empty, &composite).expect("save");
        let back = load_psd(&bytes).expect("load");
        assert_eq!(back.layer_count(), 1);
        assert_eq!(
            back.layers
                .iter()
                .next()
                .and_then(|l| l.pixmap())
                .map(|p| p.get(3, 3)),
            Some(Rgba8::rgb(1, 2, 3))
        );
    }

    #[test]
    fn rle_compressed_layers_like_photoshop_writes_decode() {
        // A 4×2 document with one RLE layer covering it, built by hand the
        // way Photoshop and Clip Studio write them.
        let (w, h) = (4usize, 2usize);
        let channel = |values: [u8; 4]| -> Vec<u8> {
            // Row 1: a literal run; row 2: a repeat run of the first value.
            let mut out = Vec::new();
            out.extend_from_slice(&1u16.to_be_bytes());
            let row1 = [3u8, values[0], values[1], values[2], values[3]];
            let row2 = [(-3i8) as u8, values[0]];
            out.extend_from_slice(&(row1.len() as u16).to_be_bytes());
            out.extend_from_slice(&(row2.len() as u16).to_be_bytes());
            out.extend_from_slice(&row1);
            out.extend_from_slice(&row2);
            out
        };
        let channels = [
            (-1i16, channel([255, 255, 128, 0])),
            (0, channel([10, 20, 30, 40])),
            (1, channel([50, 60, 70, 80])),
            (2, channel([90, 100, 110, 120])),
        ];
        let mut info = Writer { out: Vec::new() };
        info.u16(1);
        info.i32(0);
        info.i32(0);
        info.i32(h as i32);
        info.i32(w as i32);
        info.u16(4);
        for (id, data) in &channels {
            info.u16(*id as u16);
            info.u32(data.len() as u32);
        }
        info.bytes(b"8BIMnorm");
        info.u8(255);
        info.u8(0);
        info.u8(0);
        info.u8(0);
        info.u32(4 + 4 + 4);
        info.u32(0);
        info.u32(0);
        info.bytes(&[1, b'L', 0, 0]);
        for (_, data) in &channels {
            info.bytes(data);
        }
        let mut file = Writer { out: Vec::new() };
        file.bytes(b"8BPS");
        file.u16(1);
        file.bytes(&[0; 6]);
        file.u16(4);
        file.u32(h as u32);
        file.u32(w as u32);
        file.u16(8);
        file.u16(3);
        file.u32(0);
        file.u32(0);
        file.u32(info.out.len() as u32 + 8);
        file.u32(info.out.len() as u32);
        file.bytes(&info.out);
        file.u32(0);
        file.u16(0);
        file.bytes(&vec![0; w * h * 4]);

        let doc = load_psd(&file.out).expect("load");
        let layer = doc.layers.iter().next().expect("layer");
        assert_eq!(layer.name, "L");
        let pm = layer.pixmap().expect("pixels");
        assert_eq!(pm.get(1, 0), Rgba8::new(20, 60, 100, 255));
        assert_eq!(pm.get(2, 0), Rgba8::new(30, 70, 110, 128));
        assert_eq!(
            pm.get(3, 1),
            Rgba8::new(10, 50, 90, 255),
            "the repeat run fills row two"
        );
    }
}
