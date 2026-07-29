//! Bundled post registry — ivacam-authored `.cps` posts shipped in the
//! binary via `include_str!`.
//!
//! LICENSE AUDIT NOTE: everything under `posts/` is written from
//! scratch for this runtime (GPL-3.0-or-later, like the rest of
//! ivacam). Autodesk material under `refs/` is a test oracle only and
//! never registered here.

/// One post shipped with ivacam.
#[derive(Debug, Clone, Copy)]
pub struct BundledPost {
    /// Stable id the wire selection references (`CpsPostSource::Bundled`).
    pub id: &'static str,
    /// The `.cps` source text.
    pub source: &'static str,
}

/// Every bundled post, in display order.
pub const BUNDLED: &[BundledPost] = &[BundledPost {
    id: "grbl",
    source: include_str!("../posts/grbl.cps"),
}];

/// Look a bundled post up by id.
#[must_use]
pub fn bundled(id: &str) -> Option<&'static BundledPost> {
    BUNDLED.iter().find(|post| post.id == id)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every bundled post must parse, expose sane metadata, and be
    /// inspectable — a broken shipped post is a build error, not a
    /// field report.
    #[test]
    fn bundled_posts_inspect_cleanly() {
        for post in BUNDLED {
            let meta = crate::inspect_post(post.source, &format!("{}.cps", post.id))
                .unwrap_or_else(|e| panic!("bundled post {} failed inspect: {e}", post.id));
            assert!(
                !meta.description.is_empty(),
                "bundled post {} needs a description",
                post.id
            );
            assert!(
                !meta.extension.is_empty(),
                "bundled post {} needs an extension",
                post.id
            );
        }
        assert!(bundled("grbl").is_some());
        assert!(bundled("nope").is_none());
    }
}
