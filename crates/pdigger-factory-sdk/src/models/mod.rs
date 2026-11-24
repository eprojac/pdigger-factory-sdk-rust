//! Typed models of every schema in `spec/openapi.yaml` (`components/schemas`).
//!
//! Parsing is tolerant: unknown fields are kept in `extra`, missing fields take their defaults,
//! string enums keep values this SDK version does not know (`Other`), and the fields the public
//! recordings redact (face joints, appearance masks, depth textures) are optional. Integers of
//! the spec (`g`, `cursor`, `session`, ids) are `i64`.

mod catalog;
mod frame;
mod meta;
mod render;
mod session;
mod world;

pub use catalog::*;
pub use frame::*;
pub use meta::*;
pub use render::*;
pub use session::*;
pub use world::*;

use crate::wire::wire_enum;

/// Unknown fields of a model, kept as sent.
pub type Extra = serde_json::Map<String, serde_json::Value>;

/// A field that is null on the wire counts as missing (spec 2.3 rule 9): its default.
pub(crate) fn null_as_default<'de, D, T>(deserializer: D) -> Result<T, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Default + serde::Deserialize<'de>,
{
    Ok(<Option<T> as serde::Deserialize>::deserialize(deserializer)?.unwrap_or_default())
}

wire_enum! {
    /// A layer of a grid frame.
    Layer {
        /// The model's own packet (`sip1`).
        Raw = "raw",
        /// 2-D per packet (`ovl1`).
        Overlay = "overlay",
        /// 3-D per packet (`scn1`).
        Scene = "scene",
        /// The hand cards (`crd1`).
        Cards = "cards",
        /// The Stable sim's persistent world (`stw1`).
        World = "world",
    }
}

impl Layer {
    /// All five layers.
    pub const ALL: [Layer; 5] = [Layer::Raw, Layer::Overlay, Layer::Scene, Layer::Cards, Layer::World];
}

/// Comma-joined layer names, as the `layers` query parameter wants them.
pub fn join_layers(layers: &[Layer]) -> String {
    layers.iter().map(crate::wire::WireEnum::as_str).collect::<Vec<_>>().join(",")
}

wire_enum! {
    /// A hand side: `left`, `right` or `both`.
    Side {
        /// The left hand.
        Left = "left",
        /// The right hand.
        Right = "right",
        /// Both hands.
        Both = "both",
    }
}

wire_enum! {
    /// Number precision of world replies.
    Precision {
        /// outline .01 px, solid 1e-4, q 1e-5 (the default).
        Rounded = "rounded",
        /// Unrounded, for parity runs.
        Full = "full",
    }
}

wire_enum! {
    /// Format of a stream.
    StreamFormat {
        /// Server-sent events (with ids, so resumable by `Last-Event-ID`).
        Sse = "sse",
        /// One JSON object per line.
        Ndjson = "ndjson",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::wire::tests::round_trip;

    #[test]
    fn enums_round_trip() {
        round_trip::<Layer>();
        round_trip::<Side>();
        round_trip::<Precision>();
        round_trip::<StreamFormat>();
        round_trip::<EventKind>();
        round_trip::<HandStateKind>();
        round_trip::<StoneMode>();
        round_trip::<StoneFamily>();
        round_trip::<StoneShape>();
        round_trip::<Motion>();
        round_trip::<SourceKind>();
        round_trip::<ChangedBy>();
        round_trip::<WorkerState>();
        round_trip::<StableState>();
        round_trip::<LivePhase>();
        round_trip::<CardTone>();
        round_trip::<RenderMode>();
        round_trip::<RenderDraw>();
        round_trip::<RenderSurface>();
        round_trip::<Raster>();
        assert_eq!(join_layers(&[Layer::World, Layer::Cards]), "world,cards");
        assert_eq!(join_layers(&Layer::ALL), "raw,overlay,scene,cards,world");
    }
}
