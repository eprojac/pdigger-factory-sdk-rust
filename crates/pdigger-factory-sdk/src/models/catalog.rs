//! The catalog: the sources the shared session can show (the recordings and the live cameras).

use serde::{Deserialize, Serialize};

use super::Extra;
use crate::wire::wire_enum;

wire_enum! {
    /// What a source is.
    SourceKind {
        /// A recording.
        Recording = "recording",
        /// A live camera of the plant (builds `claude-20261120-live2` and later list them).
        Camera = "camera",
    }
}

/// The source camera: intrinsics and pose in the template gauge.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Camera {
    /// Intrinsics, 3×3.
    #[serde(rename = "K")]
    pub k: Vec<Vec<f64>>,
    /// Camera-to-world rotation, 3×3.
    #[serde(rename = "R_c2w")]
    pub r_c2w: Vec<Vec<f64>>,
    /// Camera-to-world translation.
    pub t_c2w: Vec<f64>,
    /// The gauge (`template (MHR adult), not metric`).
    pub gauge: Option<String>,
    /// Whether the camera was calibrated.
    pub calibrated: Option<bool>,
    /// Unknown fields.
    #[serde(flatten)]
    pub extra: Extra,
}

/// A source of the shared session: a recording, or a live camera of the plant.
///
/// A camera (`kind` `camera`, `live` true, `seekable` false) has no duration, no frame count, no
/// default start and no proxy video: each of its frames comes with the picture it was interpreted
/// from ([`crate::Session::picture`]). An entry without `kind`, `live` or `seekable` (the unit's
/// recordings omit them) is a seekable recording; [`VideoEntry::kind`], [`VideoEntry::is_live`] and
/// [`VideoEntry::is_seekable`] fill a missing one from the others (`kind` `camera` alone: live,
/// not seekable).
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct VideoEntry {
    /// Catalog id (`dry1`, ...; a camera's serial, such as `CAM00000001`).
    pub id: String,
    /// Title.
    pub title: Option<String>,
    /// Subtitle.
    pub subtitle: Option<String>,
    /// Duration in seconds; `None` for a camera.
    pub duration: Option<f64>,
    /// Source frame rate.
    pub fps: f64,
    /// Source frames; `None` for a camera.
    pub frames: Option<i64>,
    /// Source frames per grid frame.
    pub grid_step: i64,
    /// Where a select starts; `None` for a camera.
    pub default_start: Option<f64>,
    /// Warm-up hold after a select, in seconds (a camera's warm-up: 3).
    pub hold: Option<f64>,
    /// URL of the proxy video (empty for a camera).
    pub proxy: String,
    /// Proxy video size `[w, h]`.
    pub proxy_wh: Vec<i64>,
    /// Native (model crop) size `[w, h]`.
    pub native_wh: Vec<i64>,
    /// Source size `[w, h]`.
    pub source_wh: Option<Vec<i64>>,
    /// Crop `[x, y, w, h]` of the source.
    pub crop: Option<Vec<i64>>,
    /// Encoder profile.
    pub profile: Option<String>,
    /// The camera.
    pub camera: Option<Camera>,
    /// Held-out intervals `[t0, t1]`.
    pub held_out: Vec<Vec<f64>>,
    /// The stone model of this source.
    pub stone_model: Option<serde_json::Value>,
    /// `camera` for a live camera; absent means a recording.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub kind: Option<SourceKind>,
    /// True for a live camera.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub live: Option<bool>,
    /// False for a live camera.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub seekable: Option<bool>,
    /// A camera: the plant station its rig belongs to (for example `Plant A`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub station: Option<String>,
    /// Unknown fields.
    #[serde(flatten)]
    pub extra: Extra,
}

/// A source of the shared session (the API calls them videos).
pub type Source = VideoEntry;

impl VideoEntry {
    /// The kind, `recording` when the server does not say.
    pub fn kind(&self) -> SourceKind {
        match &self.kind {
            Some(kind) => kind.clone(),
            None if self.live == Some(true) => SourceKind::Camera,
            None => SourceKind::Recording,
        }
    }

    /// A live camera: no duration, no seek, rate 1.0.
    pub fn is_live(&self) -> bool {
        self.live.unwrap_or(self.kind == Some(SourceKind::Camera))
    }

    /// Whether a select may give a time and a seek is allowed.
    pub fn is_seekable(&self) -> bool {
        self.seekable.unwrap_or(!self.is_live())
    }

    /// The title, else the id.
    pub fn name(&self) -> &str {
        self.title.as_deref().unwrap_or(&self.id)
    }
}

/// `GET /api/v2/videos`: the public catalog.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct VideoList {
    /// Catalog schema.
    pub schema: String,
    /// Title.
    pub title: Option<String>,
    /// The source a new session starts on.
    pub default_video: Option<String>,
    /// The model.
    pub model: Option<serde_json::Value>,
    /// The sources.
    pub videos: Vec<VideoEntry>,
    /// Unknown fields.
    #[serde(flatten)]
    pub extra: Extra,
}

impl VideoList {
    /// The source with this id.
    pub fn get(&self, id: &str) -> Option<&VideoEntry> {
        self.videos.iter().find(|v| v.id == id)
    }

    /// The first live camera, if the catalog lists one.
    pub fn live(&self) -> Option<&VideoEntry> {
        self.videos.iter().find(|v| v.is_live())
    }

    /// The live cameras, in catalog order.
    pub fn cameras(&self) -> Vec<&VideoEntry> {
        self.videos.iter().filter(|v| v.is_live()).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recording_and_camera_catalog_entries() {
        let rec: VideoEntry = serde_json::from_str(r#"{"id":"dry1","title":"Dry side","duration":1807.2,"fps":10.0,"frames":18122,"grid_step":2,"proxy":"/p","proxy_wh":[960,540],"native_wh":[1920,1080]}"#).unwrap();
        assert_eq!(rec.kind(), SourceKind::Recording);
        assert!(!rec.is_live());
        assert!(rec.is_seekable());
        assert_eq!(rec.name(), "Dry side");
        let live: VideoEntry = serde_json::from_str(r#"{"id":"live","kind":"camera","live":true,"seekable":false,"duration":null,"frames":null,"default_start":null,"held_out":[]}"#).unwrap();
        assert_eq!(live.kind(), SourceKind::Camera);
        assert!(live.is_live());
        assert!(!live.is_seekable());
        assert_eq!(live.name(), "live");
        assert_eq!(live.duration, None);
        let implied = VideoEntry { live: Some(true), ..VideoEntry::default() };
        assert_eq!(implied.kind(), SourceKind::Camera);
        assert!(!implied.is_seekable());
        let by_kind = VideoEntry { kind: Some(SourceKind::Camera), ..VideoEntry::default() };
        assert!(by_kind.is_live());
        let list = VideoList { videos: vec![rec.clone(), live.clone()], ..VideoList::default() };
        assert_eq!(list.get("dry1"), Some(&rec));
        assert_eq!(list.get("nope"), None);
        assert_eq!(list.live(), Some(&live));
        assert_eq!(list.cameras(), [&live]);
        assert_eq!(VideoList::default().live(), None);
        assert!(VideoList::default().cameras().is_empty());
        let plant: VideoEntry = serde_json::from_str(
            r#"{"id":"CAM00000001","kind":"camera","live":true,"seekable":false,"station":"Plant A","proxy":""}"#,
        )
        .unwrap();
        assert_eq!((plant.station.as_deref(), plant.is_live(), plant.extra.len()), (Some("Plant A"), true, 0));
        let json = serde_json::to_value(&rec).unwrap();
        assert!(json.get("kind").is_none());
    }
}
