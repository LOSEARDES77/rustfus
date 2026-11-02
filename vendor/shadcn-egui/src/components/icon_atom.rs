//! Crate icons as egui atoms (rules, Media and icons; D-129):
//! `IconAtom::new(icon)` goes wherever egui takes `IntoAtoms` (an egui
//! `Button`, a crate widget's content).
//!
//! An icon atom is an image atom whose URI (`shadcn-icon://<n>`) names an
//! entry of a process-wide icon registry. The entry records the icon, its
//! side, whether the caller gave a side ([`IconAtom::size`]) and whether
//! the caller gave a colour ([`IconAtom::tint`]), so a crate widget
//! re-tints only an atom without a colour, in its recipe's state colour,
//! as a typed `.icon` slot (conformance 2.3), and sizes only an atom
//! without a side, to its recipe's icon size, as upstream sizes an svg
//! without a `size-*` class (`[&_svg:not([class*='size-'])]:size-*`). In an egui host an
//! atom without a caller colour draws in the theme's text colour, which
//! the loader bakes in; [`crate::Theme::apply`] makes it draw again on a
//! theme change. egui gives an image atom no colour of its `Ui`, so a
//! text colour the host `Ui` overrides does not reach it: give such an
//! atom [`IconAtom::tint`]. The image loader is installed by [`crate::Theme::apply`];
//! it rasterises at the size egui asks for. The registry only grows, one
//! small entry per distinct icon, side and colour mode.

use crate::components::icon::{Icon, IconShape};
use crate::components::icon_raster;
use egui::load::{ImageLoadResult, ImageLoader, ImagePoll, LoadError, SizeHint};
use egui::{Color32, ColorImage, Context, Image, ImageSource, Vec2};
use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};

/// The URI scheme of icon atoms.
const SCHEME: &str = "shadcn-icon://";
const LOADER_ID: &str = "shadcn_egui::icon_atom";

/// What makes two registered entries the same: the icon's name, size and
/// path data address (as the raster cache keys), the atom's side and
/// whether the caller gave a side and a colour.
type Key = (&'static str, u32, (usize, usize, u32, u32), u32, bool, bool);

fn key(e: &Entry) -> Key {
    let icon = &e.icon;
    let shape = match icon.shape {
        IconShape::Segs(s) => (s.as_ptr() as usize, s.len(), 0, 0),
        IconShape::Blob(r) => (r.blob.as_ptr() as usize, r.blob.len(), r.start, r.len),
    };
    (
        icon.name,
        icon.size.to_bits(),
        shape,
        e.side.to_bits(),
        e.sized,
        e.tinted,
    )
}

/// One registered icon atom.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Entry {
    icon: Icon,
    side: f32,
    /// The caller gave the side: crate widgets keep it.
    sized: bool,
    /// The caller gave a colour: the raster is a white mask under the
    /// image's tint, and crate widgets keep it.
    tinted: bool,
}

#[derive(Default)]
struct Registry {
    index: HashMap<Key, usize>,
    /// Each entry with its URI, leaked once: the registry only grows, and
    /// an atom then borrows its URI instead of building it every frame.
    entries: Vec<(Entry, &'static str)>,
}

fn registry() -> &'static Mutex<Registry> {
    static REGISTRY: OnceLock<Mutex<Registry>> = OnceLock::new();
    REGISTRY.get_or_init(Default::default)
}

/// The URI of `e`, registering it on first use.
fn register(e: Entry) -> &'static str {
    let mut r = registry().lock().unwrap_or_else(|e| e.into_inner());
    let k = key(&e);
    if let Some(&n) = r.index.get(&k) {
        return r.entries[n].1;
    }
    let n = r.entries.len();
    let uri: &'static str = format!("{SCHEME}{n}").leak();
    r.entries.push((e, uri));
    r.index.insert(k, n);
    uri
}

/// The entry a URI of the icon scheme names.
fn lookup(uri: &str) -> Option<Entry> {
    let n: usize = uri.strip_prefix(SCHEME)?.parse().ok()?;
    let r = registry().lock().unwrap_or_else(|e| e.into_inner());
    r.entries.get(n).map(|&(e, _)| e)
}

/// The image of `e`, at its side, tinted with `tint` if given.
fn image(e: Entry, tint: Option<Color32>) -> Image<'static> {
    let image =
        Image::new(ImageSource::Uri(register(e).into())).fit_to_exact_size(Vec2::splat(e.side));
    match tint {
        Some(c) => image.tint(c),
        None => image,
    }
}

/// A crate [`Icon`] as an egui atom:
/// `ui.add(egui::Button::new((IconAtom::new(icons.check), "Done")))`.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
#[must_use = "Convert it into an atom"]
pub struct IconAtom {
    icon: Icon,
    size: Option<f32>,
    tint: Option<Color32>,
}

impl IconAtom {
    /// `icon` in the widget's colour, at the widget's icon size: a crate
    /// widget's recipe size, the default icon size (`Metrics::icon`) in
    /// any other widget.
    #[inline]
    pub fn new(icon: Icon) -> Self {
        Self {
            icon,
            size: None,
            tint: None,
        }
    }

    /// The side of the icon's square in points, also inside a crate
    /// widget.
    #[inline]
    pub fn size(mut self, size: f32) -> Self {
        self.size = Some(size);
        self
    }

    /// The colour the icon is drawn in, also inside a crate widget.
    #[inline]
    pub fn tint(mut self, color: impl Into<Color32>) -> Self {
        self.tint = Some(color.into());
        self
    }
}

impl From<IconAtom> for egui::Atom<'_> {
    fn from(a: IconAtom) -> Self {
        let entry = Entry {
            icon: a.icon,
            side: a.size.unwrap_or(crate::Metrics::default().icon),
            sized: a.size.is_some(),
            tinted: a.tint.is_some(),
        };
        egui::Atom::from(image(entry, a.tint))
    }
}

/// `image` in `color`, a crate widget's state colour, if it is an icon
/// atom without a caller colour; any other image unchanged.
pub(crate) fn retint(img: Image<'_>, color: Color32) -> Image<'_> {
    match img.uri().and_then(lookup) {
        Some(e) if !e.tinted => image(Entry { tinted: true, ..e }, Some(color)),
        _ => img,
    }
}

/// `image` at `side`, a crate widget's icon size, if it is an icon atom
/// without a caller side; any other image unchanged.
pub(crate) fn resize(img: Image<'_>, side: f32) -> Image<'_> {
    match img.uri().and_then(lookup) {
        Some(e) if !e.sized && e.side != side => {
            let tint = e.tinted.then(|| img.image_options().tint);
            image(Entry { side, ..e }, tint)
        }
        _ => img,
    }
}

/// Install the icon-atom image loader on `ctx` once, and make atoms in
/// the theme's text colour draw again in the current one.
pub(crate) fn install(ctx: &Context) {
    if !ctx.is_loader_installed(LOADER_ID) {
        ctx.add_image_loader(Arc::new(IconLoader::default()));
    }
    let untinted: Vec<&'static str> = {
        let r = registry().lock().unwrap_or_else(|e| e.into_inner());
        r.entries
            .iter()
            .filter(|(e, _)| !e.tinted)
            .map(|&(_, uri)| uri)
            .collect()
    };
    for uri in untinted {
        ctx.forget_image(uri);
    }
}

/// Rasterises icon atoms at the size egui asks for.
#[derive(Default)]
struct IconLoader {
    cache: Mutex<HashMap<(String, SizeHint, Color32), Arc<ColorImage>>>,
}

impl ImageLoader for IconLoader {
    fn id(&self) -> &str {
        LOADER_ID
    }

    fn load(&self, ctx: &Context, uri: &str, size_hint: SizeHint) -> ImageLoadResult {
        if !uri.starts_with(SCHEME) {
            return Err(LoadError::NotSupported);
        }
        let entry = lookup(uri).ok_or(LoadError::NotSupported)?;
        // An untinted atom bakes the text colour in; a tinted one is a white
        // mask under its tint.
        let color = if entry.tinted {
            // token-ok: a coverage mask, tinted by the atom's colour when drawn
            Color32::WHITE
        } else {
            ctx.global_style().visuals.text_color()
        };
        let k = (uri.to_owned(), size_hint, color);
        if let Some(image) = self.cache.lock().unwrap_or_else(|e| e.into_inner()).get(&k) {
            return Ok(ImagePoll::Ready {
                image: image.clone(),
            });
        }
        let icon = entry.icon;
        let (w, h) = pixels(icon.size, size_hint);
        let stroke = crate::Theme::current(ctx).metrics.icon_stroke;
        let scale = w as f32 / icon.size;
        let alpha = icon_raster::coverage(&icon, w, h, scale, stroke * scale);
        let image = Arc::new(ColorImage::new(
            [w, h],
            alpha
                .into_iter()
                .map(|a| color.gamma_multiply_u8(a))
                .collect(),
        ));
        self.cache
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(k, image.clone());
        Ok(ImagePoll::Ready { image })
    }

    fn forget(&self, uri: &str) {
        self.cache
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .retain(|(u, _, _), _| u != uri);
    }

    fn forget_all(&self) {
        self.cache.lock().unwrap_or_else(|e| e.into_inner()).clear();
    }

    fn byte_size(&self) -> usize {
        self.cache
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .values()
            .map(|i| i.pixels.len() * std::mem::size_of::<Color32>())
            .sum()
    }
}

/// The pixel size egui asks for, the icon's view box being its original
/// size; at least 1 and at most the raster limit.
fn pixels(original: f32, hint: SizeHint) -> (usize, usize) {
    let side = match hint {
        SizeHint::Scale(s) => original * s.0,
        SizeHint::Width(w) => w as f32,
        SizeHint::Height(h) => h as f32,
        SizeHint::Size { width, height, .. } => width.min(height) as f32,
    };
    let side = (side.round() as usize).clamp(1, icon_raster::MAX_SIDE);
    (side, side)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(icon: Icon, tinted: bool) -> Entry {
        Entry {
            icon,
            side: 16.0,
            sized: false,
            tinted,
        }
    }

    #[test]
    fn registry_finds_each_entry_again() {
        let icons = crate::icons::lucide::SLOTS;
        let a = register(entry(icons.check, false));
        let b = register(entry(icons.close, false));
        let c = register(entry(icons.check, true));
        assert!(a != b && a != c);
        assert_eq!(register(entry(icons.check, false)), a);
        assert_eq!(lookup(b), Some(entry(icons.close, false)));
        assert_eq!(lookup("shadcn-icon://x"), None);
        assert_eq!(lookup("file://1"), None);
    }

    #[test]
    fn loader_rasterises_at_the_asked_size() {
        let ctx = Context::default();
        let loader = IconLoader::default();
        let hint = SizeHint::Size {
            width: 32,
            height: 32,
            maintain_aspect_ratio: true,
        };
        let check = crate::icons::lucide::SLOTS.check;
        let load = |tinted| {
            let Ok(ImagePoll::Ready { image }) =
                loader.load(&ctx, register(entry(check, tinted)), hint)
            else {
                panic!("icon not loaded")
            };
            image
        };
        let (mask, text) = (load(true), load(false));
        assert_eq!(mask.size, [32, 32]);
        let inked = |i: &ColorImage| *i.pixels.iter().max_by_key(|p| p.a()).expect("pixels");
        assert_eq!(inked(&mask), Color32::WHITE);
        // An untinted atom bakes in the text colour.
        let ink = ctx.global_style().visuals.text_color();
        assert_eq!(inked(&text), ink);
        assert!(loader.byte_size() > 0);
        assert!(matches!(
            loader.load(&ctx, "https://x", hint),
            Err(LoadError::NotSupported)
        ));
    }

    #[test]
    fn crate_widgets_retint_only_atoms_without_a_caller_colour() {
        let icons = crate::icons::lucide::SLOTS;
        // token-ok: test colours
        let (red, white) = (Color32::RED, Color32::WHITE);
        let mut atoms = egui::Atoms::new((
            IconAtom::new(icons.check),
            // A caller's white is a colour too.
            IconAtom::new(icons.close).tint(white),
            "x",
        ));
        atoms.map_images(|i| retint(i, red));
        let looks: Vec<(Color32, bool)> = atoms
            .iter_images()
            .map(|i| {
                let e = lookup(i.uri().expect("uri")).expect("entry");
                (i.image_options().tint, e.tinted)
            })
            .collect();
        assert_eq!(looks, vec![(red, true), (white, true)]);
    }

    /// A crate widget sizes only atoms without a caller side, and keeps
    /// a caller colour while it does.
    #[test]
    fn crate_widgets_resize_only_atoms_without_a_caller_side() {
        let icons = crate::icons::lucide::SLOTS;
        // token-ok: test colours
        let (blue, white) = (Color32::BLUE, Color32::WHITE);
        let mut atoms = egui::Atoms::new((
            IconAtom::new(icons.check),
            IconAtom::new(icons.check).size(20.0),
            IconAtom::new(icons.close).tint(blue),
            // A caller side equal to the default is a side too.
            IconAtom::new(icons.close).size(16.0),
        ));
        atoms.map_images(|i| resize(i, 12.0));
        let looks: Vec<(f32, bool, Color32)> = atoms
            .iter_images()
            .map(|i| {
                let e = lookup(i.uri().expect("uri")).expect("entry");
                (e.side, e.tinted, i.image_options().tint)
            })
            .collect();
        assert_eq!(
            looks,
            vec![
                (12.0, false, white),
                (20.0, false, white),
                (12.0, true, blue),
                (16.0, false, white)
            ]
        );
    }

    /// An atom borrows its registered URI: converting one every frame
    /// builds no string.
    #[test]
    fn an_atom_borrows_its_uri() {
        let icon = crate::icons::lucide::SLOTS.check;
        // Two conversions share the URI's address only if they borrow it.
        let uri = |a: IconAtom| match egui::Atom::from(a).kind {
            egui::AtomKind::Image(i) => i.uri().expect("uri").as_ptr(),
            _ => panic!("not an image atom"),
        };
        assert_eq!(uri(IconAtom::new(icon)), uri(IconAtom::new(icon)));
    }
}
