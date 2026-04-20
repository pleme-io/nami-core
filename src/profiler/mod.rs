//! `(defprofiler)` — performance profile + perf-budget declaration.
//!
//! Absorbs Chrome DevTools Performance panel, Firefox Profiler,
//! Safari Timelines, and CI-time perf-budget tools (webhint,
//! Lighthouse budgets, bundlesize). Each profile declares which
//! metrics to capture, sampling rate, and named budgets with alert
//! thresholds. Composes with (definspector) for live display and
//! (defstorage) for durable records.
//!
//! ```lisp
//! (defprofiler :name           "default"
//!              :metrics        (navigation paint layout scripting memory)
//!              :sampling-hz    4
//!              :rolling-window-seconds 60
//!              :budgets        (
//!                (budget :name "largest-contentful-paint-ms"
//!                        :metric "lcp"
//!                        :warn   1800
//!                        :error  2500)
//!                (budget :name "js-bundle-bytes"
//!                        :metric "js-size"
//!                        :warn   200000
//!                        :error  350000))
//!              :store          "perf-log")
//! ```

use serde::{Deserialize, Serialize};

#[cfg(feature = "lisp")]
use tatara_lisp::DeriveTataraDomain;

/// Metric category to sample.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "kebab-case")]
pub enum MetricCategory {
    /// `navigation.timing` — dns / connect / ttfb / full load.
    Navigation,
    /// FP, FCP, LCP — paint-related timings.
    Paint,
    /// Layout phases + CLS.
    Layout,
    /// Scripting CPU time — `script-evaluation`, JIT, GC.
    Scripting,
    /// Heap size + GC frequency + document object counts.
    Memory,
    /// Network — requests, transfer size, protocol breakdown.
    Network,
    /// Input delay — FID + INP + event-loop blockage.
    Interaction,
    /// FPS + long-animation-frame counter.
    Rendering,
    /// GPU utilization — platform-dependent.
    Gpu,
    /// User-timing marks + measures + performance.mark calls.
    UserTiming,
    /// Custom named metrics produced by the host.
    Custom,
}

/// One perf budget.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct PerfBudget {
    pub name: String,
    /// Metric identifier — free-form (`lcp`, `cls`, `js-size`, …)
    /// matching the host's metric table.
    pub metric: String,
    /// Warning threshold — exceeding fires a warn-level alert.
    pub warn: f64,
    /// Error threshold — exceeding fires an error-level alert.
    pub error: f64,
    /// Which direction counts as "over budget".
    #[serde(default)]
    pub direction: BudgetDirection,
}

/// Does the metric fail when too high or too low?
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum BudgetDirection {
    /// Alert when the value exceeds the threshold (latency, size).
    OverMax,
    /// Alert when the value drops below the threshold (FPS, throughput).
    UnderMin,
}

impl Default for BudgetDirection {
    fn default() -> Self {
        Self::OverMax
    }
}

/// Severity of an evaluation.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum BudgetSeverity {
    Ok,
    Warn,
    Error,
}

impl PerfBudget {
    /// Compare a measured value against the budget thresholds.
    #[must_use]
    pub fn evaluate(&self, value: f64) -> BudgetSeverity {
        match self.direction {
            BudgetDirection::OverMax => {
                if value >= self.error {
                    BudgetSeverity::Error
                } else if value >= self.warn {
                    BudgetSeverity::Warn
                } else {
                    BudgetSeverity::Ok
                }
            }
            BudgetDirection::UnderMin => {
                if value <= self.error {
                    BudgetSeverity::Error
                } else if value <= self.warn {
                    BudgetSeverity::Warn
                } else {
                    BudgetSeverity::Ok
                }
            }
        }
    }
}

/// Profiler profile.
#[cfg_attr(feature = "lisp", derive(DeriveTataraDomain))]
#[cfg_attr(feature = "lisp", tatara(keyword = "defprofiler"))]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ProfilerSpec {
    pub name: String,
    /// Metric categories to capture. Empty = every category.
    #[serde(default)]
    pub metrics: Vec<MetricCategory>,
    /// Sampling frequency (Hz). Clamped `[1, 240]` at apply time.
    #[serde(default = "default_sampling_hz")]
    pub sampling_hz: u32,
    /// Rolling-window size in seconds for percentile calculations.
    #[serde(default = "default_rolling_window")]
    pub rolling_window_seconds: u32,
    /// Named `(defstorage)` that receives per-sample records. Empty
    /// = no persistence (live dashboard only).
    #[serde(default)]
    pub store: Option<String>,
    /// Perf budgets attached to this profile.
    #[serde(default)]
    pub budgets: Vec<PerfBudget>,
    /// When true, only record samples that breach at least one
    /// budget's `warn` threshold (saves disk for passing runs).
    #[serde(default)]
    pub only_breaches: bool,
    #[serde(default)]
    pub description: Option<String>,
}

fn default_sampling_hz() -> u32 {
    4
}
fn default_rolling_window() -> u32 {
    60
}

const MIN_HZ: u32 = 1;
const MAX_HZ: u32 = 240;

impl ProfilerSpec {
    #[must_use]
    pub fn default_profile() -> Self {
        Self {
            name: "default".into(),
            metrics: vec![
                MetricCategory::Navigation,
                MetricCategory::Paint,
                MetricCategory::Layout,
                MetricCategory::Scripting,
                MetricCategory::Memory,
            ],
            sampling_hz: 4,
            rolling_window_seconds: 60,
            store: Some("perf-log".into()),
            budgets: Vec::new(),
            only_breaches: false,
            description: Some("Default profiler — core web vitals + scripting + memory.".into()),
        }
    }

    #[must_use]
    pub fn clamped_hz(&self) -> u32 {
        self.sampling_hz.clamp(MIN_HZ, MAX_HZ)
    }

    #[must_use]
    pub fn captures(&self, cat: MetricCategory) -> bool {
        self.metrics.is_empty() || self.metrics.contains(&cat)
    }

    /// Evaluate a measured metric against every budget that targets
    /// it. Returns the highest severity + the first breaching budget
    /// (if any).
    #[must_use]
    pub fn evaluate_metric(
        &self,
        metric: &str,
        value: f64,
    ) -> (BudgetSeverity, Option<&PerfBudget>) {
        let mut worst = BudgetSeverity::Ok;
        let mut hit: Option<&PerfBudget> = None;
        for b in &self.budgets {
            if b.metric == metric {
                let s = b.evaluate(value);
                if severity_rank(s) > severity_rank(worst) {
                    worst = s;
                    hit = Some(b);
                }
            }
        }
        (worst, hit)
    }
}

fn severity_rank(s: BudgetSeverity) -> u8 {
    match s {
        BudgetSeverity::Ok => 0,
        BudgetSeverity::Warn => 1,
        BudgetSeverity::Error => 2,
    }
}

/// Registry.
#[derive(Debug, Clone, Default)]
pub struct ProfilerRegistry {
    specs: Vec<ProfilerSpec>,
}

impl ProfilerRegistry {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    pub fn insert(&mut self, spec: ProfilerSpec) {
        self.specs.retain(|s| s.name != spec.name);
        self.specs.push(spec);
    }

    pub fn extend(&mut self, specs: impl IntoIterator<Item = ProfilerSpec>) {
        for s in specs {
            self.insert(s);
        }
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.specs.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.specs.is_empty()
    }

    #[must_use]
    pub fn specs(&self) -> &[ProfilerSpec] {
        &self.specs
    }

    #[must_use]
    pub fn get(&self, name: &str) -> Option<&ProfilerSpec> {
        self.specs.iter().find(|s| s.name == name)
    }
}

#[cfg(feature = "lisp")]
pub fn compile(src: &str) -> Result<Vec<ProfilerSpec>, String> {
    tatara_lisp::compile_typed::<ProfilerSpec>(src)
        .map_err(|e| format!("failed to compile defprofiler forms: {e}"))
}

#[cfg(feature = "lisp")]
pub fn register() {
    tatara_lisp::domain::register::<ProfilerSpec>();
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lcp_budget() -> PerfBudget {
        PerfBudget {
            name: "lcp".into(),
            metric: "lcp".into(),
            warn: 1800.0,
            error: 2500.0,
            direction: BudgetDirection::OverMax,
        }
    }

    #[test]
    fn default_profile_covers_core_web_vitals() {
        let s = ProfilerSpec::default_profile();
        assert!(s.captures(MetricCategory::Paint));
        assert!(s.captures(MetricCategory::Layout));
        assert!(s.captures(MetricCategory::Scripting));
        // Network isn't in defaults.
        assert!(!s.captures(MetricCategory::Network));
    }

    #[test]
    fn empty_metrics_list_captures_everything() {
        let s = ProfilerSpec {
            metrics: vec![],
            ..ProfilerSpec::default_profile()
        };
        assert!(s.captures(MetricCategory::Gpu));
        assert!(s.captures(MetricCategory::Custom));
    }

    #[test]
    fn clamped_hz_respects_bounds() {
        let lo = ProfilerSpec {
            sampling_hz: 0,
            ..ProfilerSpec::default_profile()
        };
        assert_eq!(lo.clamped_hz(), MIN_HZ);
        let hi = ProfilerSpec {
            sampling_hz: 9999,
            ..ProfilerSpec::default_profile()
        };
        assert_eq!(hi.clamped_hz(), MAX_HZ);
    }

    #[test]
    fn over_max_budget_evaluates_correctly() {
        let b = lcp_budget();
        assert_eq!(b.evaluate(1000.0), BudgetSeverity::Ok);
        assert_eq!(b.evaluate(2000.0), BudgetSeverity::Warn);
        assert_eq!(b.evaluate(3000.0), BudgetSeverity::Error);
    }

    #[test]
    fn under_min_budget_evaluates_correctly() {
        // FPS: warn ≤ 55, error ≤ 30.
        let b = PerfBudget {
            name: "fps".into(),
            metric: "fps".into(),
            warn: 55.0,
            error: 30.0,
            direction: BudgetDirection::UnderMin,
        };
        assert_eq!(b.evaluate(60.0), BudgetSeverity::Ok);
        assert_eq!(b.evaluate(55.0), BudgetSeverity::Warn);
        assert_eq!(b.evaluate(30.0), BudgetSeverity::Error);
    }

    #[test]
    fn evaluate_metric_returns_worst_severity_plus_hit() {
        let s = ProfilerSpec {
            budgets: vec![
                PerfBudget {
                    name: "lcp-soft".into(),
                    metric: "lcp".into(),
                    warn: 2000.0,
                    error: 3500.0,
                    direction: BudgetDirection::OverMax,
                },
                PerfBudget {
                    name: "lcp-strict".into(),
                    metric: "lcp".into(),
                    warn: 1500.0,
                    error: 2000.0,
                    direction: BudgetDirection::OverMax,
                },
            ],
            ..ProfilerSpec::default_profile()
        };
        let (sev, hit) = s.evaluate_metric("lcp", 2500.0);
        assert_eq!(sev, BudgetSeverity::Error);
        assert_eq!(hit.unwrap().name, "lcp-strict");
    }

    #[test]
    fn evaluate_metric_returns_ok_when_no_budget_matches() {
        let s = ProfilerSpec {
            budgets: vec![lcp_budget()],
            ..ProfilerSpec::default_profile()
        };
        let (sev, hit) = s.evaluate_metric("fps", 55.0);
        assert_eq!(sev, BudgetSeverity::Ok);
        assert!(hit.is_none());
    }

    #[test]
    fn registry_dedupes_by_name() {
        let mut reg = ProfilerRegistry::new();
        reg.insert(ProfilerSpec::default_profile());
        reg.insert(ProfilerSpec {
            sampling_hz: 10,
            ..ProfilerSpec::default_profile()
        });
        assert_eq!(reg.len(), 1);
        assert_eq!(reg.specs()[0].sampling_hz, 10);
    }

    #[test]
    fn budget_roundtrips_through_serde() {
        let s = ProfilerSpec {
            budgets: vec![lcp_budget()],
            ..ProfilerSpec::default_profile()
        };
        let json = serde_json::to_string(&s).unwrap();
        let back: ProfilerSpec = serde_json::from_str(&json).unwrap();
        assert_eq!(back.budgets.len(), 1);
        assert_eq!(back.budgets[0].name, "lcp");
    }

    #[test]
    fn metric_category_roundtrip() {
        for c in [
            MetricCategory::Navigation,
            MetricCategory::Paint,
            MetricCategory::UserTiming,
            MetricCategory::Custom,
        ] {
            let s = ProfilerSpec {
                metrics: vec![c],
                ..ProfilerSpec::default_profile()
            };
            let json = serde_json::to_string(&s).unwrap();
            let back: ProfilerSpec = serde_json::from_str(&json).unwrap();
            assert_eq!(back.metrics, vec![c]);
        }
    }

    #[cfg(feature = "lisp")]
    #[test]
    fn compile_parses_profiler_form() {
        let src = r#"
            (defprofiler :name "default"
                         :metrics ("paint" "layout" "scripting")
                         :sampling-hz 8
                         :rolling-window-seconds 120
                         :store "perf-log")
        "#;
        let specs = compile(src).unwrap();
        assert_eq!(specs.len(), 1);
        let s = &specs[0];
        assert_eq!(s.name, "default");
        assert_eq!(s.sampling_hz, 8);
        assert!(s.captures(MetricCategory::Paint));
        assert_eq!(s.store.as_deref(), Some("perf-log"));
    }
}
