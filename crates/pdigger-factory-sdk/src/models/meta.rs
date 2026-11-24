//! Meta and ops models: the entry point, the model, the topology, health, readiness, status,
//! metrics.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use super::{Extra, Side, StableStatus, WorkerStatus};

/// The retained frame window of the current session.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Retained {
    /// The session.
    pub session: i64,
    /// Oldest retained grid frame.
    pub g0: Option<i64>,
    /// Newest retained grid frame.
    pub g1: Option<i64>,
    /// Media time of `g0`.
    pub t0: Option<f64>,
    /// Media time of `g1`.
    pub t1: Option<f64>,
    /// Number of retained frames.
    pub n: i64,
    /// Unknown fields.
    #[serde(flatten)]
    pub extra: Extra,
}

/// `GET /api/v2`: links, layer schema ids, limits and the build.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct ApiRoot {
    /// API version (`2.2.0` on 20.20, build `claude-20261122-hr4x1`).
    pub api: String,
    /// Build of the server.
    pub build: String,
    /// Changes with every process start.
    pub boot: String,
    /// `trt`, `ort`, `replay` or `none`.
    pub backend: Option<String>,
    /// Path of the OpenAPI document.
    pub spec: String,
    /// Path of the docs.
    pub docs: String,
    /// Layer name to schema id (`world` → `stw1`).
    pub layers: BTreeMap<String, String>,
    /// Render name to schema id.
    pub render: BTreeMap<String, String>,
    /// Server limits (`retained_frames`, `max_streams`, ...).
    pub limits: BTreeMap<String, serde_json::Value>,
    /// Docs policy.
    pub docs_policy: Option<serde_json::Value>,
    /// Unknown fields.
    #[serde(flatten)]
    pub extra: Extra,
}

/// One engine of the running model.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct ModelEngine {
    /// Stage name (`encoder-1920`, `hands-w4`, ...).
    pub stage: String,
    /// Engine hash.
    pub sha256: String,
    /// Engine size.
    pub bytes: Option<i64>,
    /// Hash of the ONNX it was built from.
    pub onnx_sha256: Option<String>,
    /// INT4 weights.
    pub int4_weights: Option<i64>,
    /// INT4 exceptions.
    pub int4_exceptions: Option<i64>,
    /// Unknown fields.
    #[serde(flatten)]
    pub extra: Extra,
}

/// `GET /api/v2/model`: the model that is running.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct ModelInfo {
    /// `mdl1`.
    pub schema: String,
    /// Checkpoint id.
    pub checkpoint: String,
    /// Model name.
    pub name: Option<String>,
    /// Training stage.
    pub stage: Option<String>,
    /// Inference backend.
    pub backend: String,
    /// TensorRT profile.
    pub profile: Option<String>,
    /// TensorRT version.
    pub tensorrt: Option<String>,
    /// Engines with hashes.
    pub engines: Vec<ModelEngine>,
    /// Unknown fields.
    #[serde(flatten)]
    pub extra: Extra,
}

/// Per-side joint indices.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct PerSide<T> {
    /// Left.
    pub left: T,
    /// Right.
    pub right: T,
}

impl<T> PerSide<T> {
    /// The value of `side` (`both` and unknown sides give `None`).
    pub fn get(&self, side: &Side) -> Option<&T> {
        match side {
            Side::Left => Some(&self.left),
            Side::Right => Some(&self.right),
            _ => None,
        }
    }
}

/// `GET /api/v2/topology`: skeleton topology, class names, palette and grid.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Topology {
    /// `top1`.
    pub schema: String,
    /// Number of joints (70).
    pub joints: usize,
    /// Joints that are never shown.
    pub never: Vec<usize>,
    /// Wrist joint of each hand.
    pub wrist: PerSide<usize>,
    /// Finger-tip joints of each hand.
    pub tips: PerSide<Vec<usize>>,
    /// Bones as joint index pairs.
    pub bones: Vec<[usize; 2]>,
    /// Action class names.
    pub actions: Vec<String>,
    /// Held class names.
    pub held: Vec<String>,
    /// Palette (`left`, `right`, `stones`, `focus`, ...), CSS hex colours.
    pub colors: BTreeMap<String, String>,
    /// Model grid `[rows, cols]`.
    pub grid: Vec<usize>,
    /// Unknown fields.
    #[serde(flatten)]
    pub extra: Extra,
}

impl Topology {
    /// Bones whose two joints are both shown (neither is in `never`).
    pub fn visible_bones(&self) -> Vec<[usize; 2]> {
        self.bones.iter().copied().filter(|[a, b]| !self.never.contains(a) && !self.never.contains(b)).collect()
    }

    /// The palette colour `name` as RGB, if it is a `#rrggbb` colour.
    pub fn color(&self, name: &str) -> Option<[u8; 3]> {
        parse_hex_color(self.colors.get(name)?)
    }
}

/// `#rrggbb` as RGB.
pub fn parse_hex_color(text: &str) -> Option<[u8; 3]> {
    let hex = text.strip_prefix('#')?;
    if hex.len() != 6 || !hex.is_ascii() {
        return None;
    }
    let byte = |i: usize| u8::from_str_radix(&hex[i..i + 2], 16).ok();
    Some([byte(0)?, byte(2)?, byte(4)?])
}

/// `GET /api/v2/health`: liveness.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Health {
    /// The process answers.
    pub ok: bool,
    /// Worker state.
    pub worker: String,
    /// Viewers.
    pub viewers: i64,
    /// Unknown fields.
    #[serde(flatten)]
    pub extra: Extra,
}

/// `GET /api/v2/ready`: readiness.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Ready {
    /// Ready.
    pub ready: bool,
    /// What it still waits for.
    pub waiting: Vec<String>,
    /// Unknown fields.
    #[serde(flatten)]
    pub extra: Extra,
}

/// A helper host's status.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct HostStatus {
    /// `running`, `starting`, `failed`, `off`.
    pub state: String,
    /// Restarts.
    pub restarts: i64,
    /// Resident memory.
    pub rss_mb: Option<f64>,
    /// Queue length.
    pub queue: Option<i64>,
    /// Unknown fields.
    #[serde(flatten)]
    pub extra: Extra,
}

/// `GET /api/v2/status`: everything an operator wants on one page.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Status {
    /// Build.
    pub build: String,
    /// Boot id.
    pub boot: String,
    /// Uptime in seconds.
    pub uptime_s: f64,
    /// `legacy` or `next`.
    pub page: String,
    /// The worker.
    pub worker: WorkerStatus,
    /// The retained window.
    pub retained: Option<Retained>,
    /// The Stable sim.
    pub stable: StableStatus,
    /// Helper hosts.
    pub hosts: BTreeMap<String, HostStatus>,
    /// Memory.
    pub memory: Option<serde_json::Value>,
    /// Viewers.
    pub viewers: i64,
    /// Long-polls.
    pub long_polls: Option<i64>,
    /// Streams.
    pub streams: Option<i64>,
    /// Unknown fields.
    #[serde(flatten)]
    pub extra: Extra,
}

/// `GET /metrics`: the Prometheus text and its samples.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Metrics {
    /// The text exposition as sent.
    pub text: String,
    /// Every sample line, parsed.
    pub samples: Vec<MetricSample>,
}

/// One Prometheus sample: `name{labels} value`.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct MetricSample {
    /// Metric name.
    pub name: String,
    /// Labels.
    pub labels: BTreeMap<String, String>,
    /// Value (`NaN`, `+Inf`, `-Inf` included).
    pub value: f64,
}

impl Metrics {
    /// Parses the Prometheus text exposition format (comments and lines that do not parse are
    /// skipped).
    pub fn parse(text: &str) -> Metrics {
        Metrics { text: text.to_string(), samples: text.lines().filter_map(parse_sample).collect() }
    }

    /// The samples of metric `name`.
    pub fn get<'a>(&'a self, name: &'a str) -> impl Iterator<Item = &'a MetricSample> + 'a {
        self.samples.iter().filter(move |s| s.name == name)
    }

    /// The value of the sample `name` whose labels include every `labels` pair.
    pub fn value(&self, name: &str, labels: &[(&str, &str)]) -> Option<f64> {
        self.get(name)
            .find(|s| labels.iter().all(|(k, v)| s.labels.get(*k).map(String::as_str) == Some(*v)))
            .map(|s| s.value)
    }
}

fn parse_sample(line: &str) -> Option<MetricSample> {
    let line = line.trim();
    if line.is_empty() || line.starts_with('#') {
        return None;
    }
    let name_end = line.find(|c: char| c == '{' || c.is_whitespace())?;
    let name = &line[..name_end];
    let mut rest = &line[name_end..];
    let mut labels = BTreeMap::new();
    if let Some(inner) = rest.strip_prefix('{') {
        let (parsed, after) = parse_labels(inner)?;
        labels = parsed;
        rest = after;
    }
    let value = rest.split_whitespace().next()?;
    let value = match value {
        "+Inf" | "Inf" => f64::INFINITY,
        "-Inf" => f64::NEG_INFINITY,
        other => other.parse::<f64>().ok()?,
    };
    Some(MetricSample { name: name.to_string(), labels, value })
}

/// Labels up to the closing `}`; returns them and the text after it.
fn parse_labels(text: &str) -> Option<(BTreeMap<String, String>, &str)> {
    let mut labels = BTreeMap::new();
    let mut chars = text.char_indices().peekable();
    loop {
        while chars.next_if(|(_, c)| *c == ',' || c.is_whitespace()).is_some() {}
        let (start, c) = chars.next()?;
        if c == '}' {
            return Some((labels, &text[start + 1..]));
        }
        let mut key = String::from(c);
        loop {
            let (_, c) = chars.next()?;
            if c == '=' {
                break;
            }
            key.push(c);
        }
        if chars.next()?.1 != '"' {
            return None;
        }
        let mut value = String::new();
        loop {
            match chars.next()?.1 {
                '"' => break,
                '\\' => match chars.next()?.1 {
                    'n' => value.push('\n'),
                    other => value.push(other),
                },
                other => value.push(other),
            }
        }
        labels.insert(key.trim().to_string(), value);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn metrics_parse_labels_escapes_and_specials() {
        let m = Metrics::parse(
            "# HELP x y\n\ninterp_frames_total{video=\"dry1\"} 3726\nup 1\nweird{a=\"q\\\"x\\\\y\\nz\", b=\"2\"} +Inf\nneg -Inf\nnan NaN\nbad{a=\"1\" 3\nbad2{a 1}\nbad3{a=1} 2\nnovalue\nbad4 x\n",
        );
        assert_eq!(m.samples.len(), 5);
        assert_eq!(m.value("interp_frames_total", &[("video", "dry1")]), Some(3726.0));
        assert_eq!(m.value("interp_frames_total", &[("video", "dry2")]), None);
        assert_eq!(m.value("up", &[]), Some(1.0));
        let weird = m.get("weird").next().unwrap();
        assert_eq!(weird.labels["a"], "q\"x\\y\nz");
        assert_eq!(weird.labels["b"], "2");
        assert_eq!(weird.value, f64::INFINITY);
        assert_eq!(m.value("neg", &[]), Some(f64::NEG_INFINITY));
        assert!(m.value("nan", &[]).unwrap().is_nan());
        assert!(m.text.starts_with("# HELP"));
    }

    #[test]
    fn topology_helpers() {
        let t: Topology = serde_json::from_str(
            r##"{"schema":"top1","joints":70,"never":[15],"wrist":{"left":62,"right":41},"tips":{"left":[42],"right":[21]},"bones":[[13,11],[15,16]],"colors":{"left":"#f68c36","bad":"red","odd":"#zz0000"},"grid":[40,64]}"##,
        )
        .unwrap();
        assert_eq!(t.visible_bones(), vec![[13, 11]]);
        assert_eq!(t.wrist.get(&Side::Left), Some(&62));
        assert_eq!(t.wrist.get(&Side::Right), Some(&41));
        assert_eq!(t.wrist.get(&Side::Both), None);
        assert_eq!(t.tips.get(&Side::Right), Some(&vec![21]));
        assert_eq!(t.color("left"), Some([0xf6, 0x8c, 0x36]));
        assert_eq!(t.color("bad"), None);
        assert_eq!(t.color("odd"), None);
        assert_eq!(t.color("missing"), None);
        assert_eq!(parse_hex_color("#ffé000"), None);
    }
}
