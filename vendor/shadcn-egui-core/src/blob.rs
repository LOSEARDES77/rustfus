//! Reader for the icon blobs icongen writes (`tools/icongen/src/blob.rs`
//! documents the format). Icon crates hold one blob each; an `Icon` points
//! at its record with a [`BlobRange`].
//!
//! Records start at any even offset of an `include_bytes!` blob, so words
//! are read byte pair by byte pair, never through a `&[u16]` cast. A
//! malformed record must not crash a frame: reads past its end stop the
//! strokes, drop the fill and end the layers.

use crate::icon::{IconLayer, Seg, SegIter};

const COORD_SCALE: f32 = 1024.0;
const MITER_SCALE: f32 = 8192.0;
const ALPHA_SCALE: f32 = 65535.0;

/// One icon's record inside an icon crate's blob: `blob[start..start + len]`.
#[derive(Clone, Copy)]
pub struct BlobRange {
    pub blob: &'static [u8],
    pub start: u32,
    pub len: u32,
}

impl BlobRange {
    /// The record's bytes; `None` when the range does not fit the blob.
    fn record(self) -> Option<&'static [u8]> {
        let start = self.start as usize;
        start
            .checked_add(self.len as usize)
            .and_then(|end| self.blob.get(start..end))
    }

    /// The record's bytes; empty when the range does not fit the blob.
    fn bytes(self) -> &'static [u8] {
        self.record().unwrap_or(&[])
    }
}

/// Two ranges are the same icon when their records hold the same bytes,
/// wherever they live: the linker may give each crate its own copy of a
/// blob, so addresses say nothing. Ranges that do not fit their blob are
/// equal only when blob address, start and len all match.
impl PartialEq for BlobRange {
    fn eq(&self, other: &Self) -> bool {
        match (self.record(), other.record()) {
            (Some(a), Some(b)) => a == b,
            _ => {
                core::ptr::eq(self.blob.as_ptr(), other.blob.as_ptr())
                    && self.start == other.start
                    && self.len == other.len
            }
        }
    }
}

impl core::fmt::Debug for BlobRange {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("BlobRange")
            .field("start", &self.start)
            .field("len", &self.len)
            .finish()
    }
}

/// Little-endian 16-bit reader over one record; `None` past its end.
#[derive(Clone)]
struct Reader {
    bytes: &'static [u8],
    pos: usize,
}

impl Reader {
    fn word(&mut self) -> Option<u16> {
        let pair = self.bytes.get(self.pos..self.pos + 2)?;
        self.pos += 2;
        Some(u16::from_le_bytes([pair[0], pair[1]]))
    }
    fn coord(&mut self) -> Option<f32> {
        Some(f32::from(self.word()? as i16) / COORD_SCALE)
    }
    fn miter(&mut self) -> Option<f32> {
        Some(f32::from(self.word()? as i16) / MITER_SCALE)
    }
    fn point(&mut self) -> Option<[f32; 2]> {
        Some([self.coord()?, self.coord()?])
    }
}

/// The stroke commands of one blob record, in the 24-unit box.
#[derive(Clone)]
pub struct BlobSegs {
    /// Reads only the stroke section, so an opcode cut short ends it.
    r: Reader,
}

impl BlobSegs {
    /// Strokes from a stroke section's opcodes and coordinates.
    fn new(bytes: &'static [u8]) -> Self {
        BlobSegs {
            r: Reader { bytes, pos: 0 },
        }
    }

    fn seg(&mut self) -> Option<Seg> {
        let r = &mut self.r;
        Some(match r.word()? {
            0 => Seg::M(r.coord()?, r.coord()?),
            1 => Seg::L(r.coord()?, r.coord()?),
            2 => Seg::Q(r.coord()?, r.coord()?, r.coord()?, r.coord()?),
            3 => Seg::C(
                r.coord()?,
                r.coord()?,
                r.coord()?,
                r.coord()?,
                r.coord()?,
                r.coord()?,
            ),
            4 => Seg::Z,
            // Not written by icongen.
            _ => return None,
        })
    }
}

impl core::fmt::Debug for BlobSegs {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("BlobSegs")
            .field("pos", &self.r.pos)
            .field("end", &self.r.bytes.len())
            .finish()
    }
}

impl Iterator for BlobSegs {
    type Item = Seg;

    fn next(&mut self) -> Option<Seg> {
        let seg = self.seg();
        if seg.is_none() {
            // A malformed record: stop drawing rather than crash a frame.
            self.r.pos = self.r.bytes.len();
        }
        seg
    }
}

/// A pre-triangulated fill in the 24-unit box, plus per contour each
/// vertex and its outward miter (length ≤ 2; zero where no fringe belongs).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct FillMesh {
    pub vertices: Vec<[f32; 2]>,
    pub indices: Vec<u16>,
    pub contours: Vec<Vec<([f32; 2], [f32; 2])>>,
}

/// The layers of one blob record: the opaque one, then each translucent one.
#[derive(Clone)]
pub(crate) struct BlobLayers {
    r: Reader,
    first: bool,
}

impl BlobLayers {
    pub(crate) fn new(range: BlobRange) -> Self {
        BlobLayers {
            r: Reader {
                bytes: range.bytes(),
                pos: 0,
            },
            first: true,
        }
    }

    /// The next layer, and whether the record can be read past it.
    fn layer(&mut self) -> Option<(IconLayer, bool)> {
        let r = &mut self.r;
        let alpha = if self.first {
            1.0
        } else {
            f32::from(r.word()?) / ALPHA_SCALE
        };
        self.first = false;
        let len = usize::from(r.word()?);
        let start = r.pos;
        let end = start.saturating_add(2 * len).min(r.bytes.len());
        r.pos = start.saturating_add(2 * len);
        let segs = SegIter::Words(BlobSegs::new(&r.bytes[start..end]));
        let fill = read_fill(r);
        let readable = fill.is_some();
        let layer = IconLayer {
            alpha,
            segs,
            fill: fill.flatten(),
        };
        Some((layer, readable))
    }
}

impl core::fmt::Debug for BlobLayers {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("BlobLayers")
            .field("pos", &self.r.pos)
            .field("end", &self.r.bytes.len())
            .finish()
    }
}

impl Iterator for BlobLayers {
    type Item = IconLayer;

    fn next(&mut self) -> Option<IconLayer> {
        if !self.first && self.r.pos >= self.r.bytes.len() {
            return None;
        }
        match self.layer() {
            Some((layer, readable)) => {
                if !readable {
                    self.r.pos = self.r.bytes.len();
                }
                Some(layer)
            }
            None => {
                self.r.pos = self.r.bytes.len();
                self.first = false;
                None
            }
        }
    }
}

/// A fill section at the reader; `None` when the record ends inside it,
/// `Some(None)` for an all-zero section and for a triangle index past the
/// vertices.
fn read_fill(r: &mut Reader) -> Option<Option<FillMesh>> {
    let (nv, ni, nc) = (
        usize::from(r.word()?),
        usize::from(r.word()?),
        usize::from(r.word()?),
    );
    if nv == 0 && ni == 0 && nc == 0 {
        return Some(None);
    }
    let vertices: Vec<[f32; 2]> = (0..nv).map(|_| r.point()).collect::<Option<_>>()?;
    let indices: Vec<u16> = (0..ni).map(|_| r.word()).collect::<Option<_>>()?;
    let contours: Vec<Vec<([f32; 2], [f32; 2])>> = (0..nc)
        .map(|_| {
            let len = usize::from(r.word()?);
            (0..len)
                .map(|_| Some((r.point()?, [r.miter()?, r.miter()?])))
                .collect::<Option<_>>()
        })
        .collect::<Option<_>>()?;
    if nv == 0 && nc == 0 || ni % 3 != 0 || indices.iter().any(|&i| usize::from(i) >= nv) {
        return Some(None);
    }
    Some(Some(FillMesh {
        vertices,
        indices,
        contours,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::icon::{Icon, IconLayer, IconShape, Seg};

    /// Words (as i16 values, so miters can be negative) to leaked LE bytes.
    fn leak(words: &[i32]) -> &'static [u8] {
        let bytes: Vec<u8> = words
            .iter()
            .flat_map(|&w| (w as i16 as u16).to_le_bytes())
            .collect();
        Box::leak(bytes.into_boxed_slice())
    }

    /// M(1, 2) L(3, 4) Z, then a one-triangle fill with one contour.
    fn record() -> &'static [u8] {
        leak(&[
            7, 0, 1024, 2048, 1, 3072, 4096, 4, // stroke
            3, 3, 1, // nv ni nc
            0, 0, 1024, 0, 0, 1024, // vertices
            0, 1, 2, // indices
            3, 0, 0, -8192, -8192, 1024, 0, 8192, -8192, 0, 1024, -8192, 8192, // contour
        ])
    }

    /// Every layer's strokes, in order.
    fn segs(icon: &Icon) -> Vec<Seg> {
        icon.layers().flat_map(|l| l.segs).collect()
    }

    /// The first layer's fill.
    fn fill(icon: &Icon) -> Option<FillMesh> {
        icon.layers().next().and_then(|l| l.fill)
    }

    /// Layer 1: M(1, 1), no fill. Layer 2 at 26214 / 65535 = 0.4: L(2, 2)
    /// and a one-triangle fill. Layer 3 at 0.5: Z, no fill.
    fn layered_record() -> &'static [u8] {
        leak(&[
            3, 0, 1024, 1024, 0, 0, 0, // opaque
            26214, 3, 1, 2048, 2048, 3, 3, 0, 0, 0, 1024, 0, 0, 1024, 0, 1, 2, // 0.4
            32768, 1, 4, 0, 0, 0, // 0.5
        ])
    }

    fn blob_icon(blob: &'static [u8], start: u32, len: u32) -> Icon {
        Icon {
            name: "t",
            size: 24.0,
            shape: IconShape::Blob(BlobRange { blob, start, len }),
        }
    }

    #[test]
    fn blob_record_decodes_strokes_and_fill() {
        let blob = record();
        let icon = blob_icon(blob, 0, blob.len() as u32);
        assert_eq!(
            segs(&icon),
            vec![Seg::M(1.0, 2.0), Seg::L(3.0, 4.0), Seg::Z]
        );
        let fill = fill(&icon).expect("has a fill");
        assert_eq!(fill.vertices, vec![[0.0, 0.0], [1.0, 0.0], [0.0, 1.0]]);
        assert_eq!(fill.indices, vec![0, 1, 2]);
        assert_eq!(fill.contours.len(), 1);
        assert_eq!(fill.contours[0][0], ([0.0, 0.0], [-1.0, -1.0]));
        assert_eq!(fill.contours[0][1], ([1.0, 0.0], [1.0, -1.0]));
    }

    #[test]
    fn translucent_layers_follow_the_opaque_one_with_their_alpha() {
        let blob = layered_record();
        let icon = blob_icon(blob, 0, blob.len() as u32);
        let layers: Vec<IconLayer> = icon.layers().collect();
        assert_eq!(layers.len(), 3);
        assert_eq!(layers[0].alpha, 1.0);
        assert_eq!(
            layers[0].segs.clone().collect::<Vec<_>>(),
            vec![Seg::M(1.0, 1.0)]
        );
        assert_eq!(layers[0].fill, None);
        assert_eq!(layers[1].alpha, 26214.0 / 65535.0);
        assert!((layers[1].alpha - 0.4).abs() < 1e-5);
        assert_eq!(
            layers[1].segs.clone().collect::<Vec<_>>(),
            vec![Seg::L(2.0, 2.0)]
        );
        let f = layers[1].fill.as_ref().expect("the second layer's fill");
        assert_eq!(f.indices, vec![0, 1, 2]);
        assert!((layers[2].alpha - 0.5).abs() < 1e-4);
        assert_eq!(layers[2].segs.clone().collect::<Vec<_>>(), vec![Seg::Z]);
        assert_eq!(layers[2].fill, None);
    }

    #[test]
    fn a_layer_cut_short_ends_the_layers() {
        // A second layer whose fill section is missing: its strokes are
        // drawn, nothing after it is read.
        let blob = leak(&[3, 0, 1024, 1024, 0, 0, 0, 100, 3, 1, 2048, 2048, 0]);
        let icon = blob_icon(blob, 0, blob.len() as u32);
        let layers: Vec<IconLayer> = icon.layers().collect();
        assert_eq!(layers.len(), 2);
        assert_eq!(
            layers[1].segs.clone().collect::<Vec<_>>(),
            vec![Seg::L(2.0, 2.0)]
        );
        assert_eq!(layers[1].fill, None);
        // An alpha word with nothing after it is no layer.
        let blob = leak(&[3, 0, 1024, 1024, 0, 0, 0, 100]);
        assert_eq!(blob_icon(blob, 0, blob.len() as u32).layers().count(), 1);
    }

    #[test]
    fn segs_icons_are_one_opaque_layer() {
        static SEGS: [Seg; 1] = [Seg::M(0.0, 0.0)];
        let icon = Icon {
            name: "s",
            size: 24.0,
            shape: IconShape::Segs(&SEGS),
        };
        let layers: Vec<IconLayer> = icon.layers().collect();
        assert_eq!(layers.len(), 1);
        assert_eq!(layers[0].alpha, 1.0);
    }

    #[test]
    fn records_are_read_at_their_offset() {
        // M(1, 1) with no fill (7 words = 14 bytes), then Z with no fill.
        let blob = leak(&[3, 0, 1024, 1024, 0, 0, 0, 1, 4, 0, 0, 0]);
        let second = blob_icon(blob, 14, 10);
        assert_eq!(segs(&second), vec![Seg::Z]);
        assert_eq!(fill(&second), None, "an all-zero fill section is no fill");
        assert_eq!(segs(&blob_icon(blob, 0, 14)), vec![Seg::M(1.0, 1.0)]);
    }

    #[test]
    fn segs_icons_iterate_their_slice_and_have_no_fill() {
        static SEGS: [Seg; 2] = [Seg::M(0.0, 0.0), Seg::L(1.0, 1.0)];
        let icon = Icon {
            name: "s",
            size: 24.0,
            shape: IconShape::Segs(&SEGS),
        };
        assert_eq!(segs(&icon), SEGS.to_vec());
        assert_eq!(fill(&icon), None);
    }

    #[test]
    fn unknown_opcode_ends_the_strokes_instead_of_panicking() {
        let blob = leak(&[3, 9, 0, 0, 0, 0, 0]);
        assert_eq!(segs(&blob_icon(blob, 0, 14)).len(), 0);
    }

    #[test]
    fn truncated_and_garbage_records_never_panic() {
        // Every prefix of a valid record, one layer or three, including odd
        // lengths; every layer is read to its end.
        for whole in [record(), layered_record()] {
            for len in 0..=whole.len() as u32 {
                let icon = blob_icon(whole, 0, len);
                let layers: Vec<IconLayer> = icon.layers().collect();
                assert!(layers.len() <= 3, "{len}: {} layers", layers.len());
                for layer in layers {
                    assert!((0.0..=1.0).contains(&layer.alpha));
                    assert!(layer.segs.count() <= whole.len());
                }
            }
        }
        let whole = record();
        // Ranges that point past the end of the blob.
        for (start, len) in [(0, 1000), (40, 20), (u32::MAX, 2), (2, u32::MAX)] {
            let icon = blob_icon(whole, start, len);
            let _ = segs(&icon).len();
            let _ = fill(&icon);
        }
        // Fixed garbage: huge counts, odd lengths, opcodes cut short.
        let garbage: [&[i32]; 6] = [
            &[-1],
            &[-1, 0, 0],
            &[2, 3, 0],
            &[0, -1, -1, -1, 0, 0],
            &[0, 1, 0, 0, 7, 0x7fff, 0, 0],
            &[1, 4, 0, 0, 1, 2, 0, 0, 0, 0, 0, 0, 0, 0],
        ];
        for words in garbage {
            let blob = leak(words);
            for len in [blob.len() as u32, blob.len() as u32 - 1] {
                let icon = blob_icon(blob, 0, len);
                assert!(segs(&icon).len() <= blob.len());
                let _ = fill(&icon);
            }
        }
    }

    #[test]
    fn a_triangle_index_past_the_vertices_drops_the_fill() {
        // One vertex, one triangle 0 1 2, no contours.
        let blob = leak(&[0, 1, 3, 0, 0, 0, 0, 1, 2]);
        assert_eq!(fill(&blob_icon(blob, 0, blob.len() as u32)), None);
    }

    #[test]
    fn blob_ranges_compare_by_record_bytes_and_print_without_bytes() {
        let blob = record();
        let a = BlobRange {
            blob,
            start: 0,
            len: 8,
        };
        assert_eq!(a, a);
        assert_ne!(a, BlobRange { start: 2, ..a }, "another record");
        assert_ne!(a, BlobRange { len: 10, ..a }, "a longer record");
        // The same record in another blob (another crate's copy, or the
        // same bytes at another offset) is the same icon.
        let copy: &'static [u8] = Box::leak(blob.to_vec().into_boxed_slice());
        assert_eq!(a, BlobRange { blob: copy, ..a });
        let shifted = leak(&[9, 7, 0, 1024, 2048]);
        assert_eq!(
            a,
            BlobRange {
                blob: shifted,
                start: 2,
                len: 8
            }
        );
        assert_eq!(format!("{a:?}"), "BlobRange { start: 0, len: 8 }");
    }

    #[test]
    fn out_of_range_blob_ranges_compare_by_position() {
        let blob = record();
        let bad = BlobRange {
            blob,
            start: u32::MAX,
            len: 8,
        };
        assert_eq!(bad, bad);
        let copy: &'static [u8] = Box::leak(blob.to_vec().into_boxed_slice());
        assert_ne!(bad, BlobRange { blob: copy, ..bad }, "another blob");
        assert_ne!(bad, BlobRange { start: 0, ..bad }, "an in-range record");
        let empty = BlobRange {
            blob,
            start: 0,
            len: 0,
        };
        assert_ne!(
            BlobRange { len: 0, ..bad },
            empty,
            "out of range is not an empty record"
        );
    }
}
