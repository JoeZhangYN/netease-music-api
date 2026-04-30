// file-size-gate: exempt PR-4 — Quality SOT 单文件包含 enum + 8 变体 helpers + serde + tests，按职责单一不应拆分
// invariants-uplift: exempt PR-4 — 8-arm match IS the typestate uplift this linter recommends
// test-gate: exempt PR-4 — display_name_zh 直接测试 + 通过 quality_display_name 间接覆盖；DEFAULT_QUALITY 是 const 不需 test

//! Music quality enum + wire-format compat shims.
//!
//! `Quality` enum is the SOT (PR-4). Existing string-based constants
//! (`VALID_QUALITIES`, `quality_display_name`, `DEFAULT_QUALITY`) are
//! derived/co-listed compat shims for backward compat — PR-6/PR-7 will
//! migrate consumers to the enum directly and remove the shims.
//!
//! Pre-PR-4 the project listed 7-of-8 qualities in `info.rs` (missing
//! `dolby`); the enum's `ALL` const + exhaustive match on `wire_str`
//! makes that drift impossible going forward (compile-time enforcement).

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};

/// Audio quality variants. SOT for the 8-quality domain.
/// Wire format: `#[serde(rename_all = "lowercase")]` keeps existing JSON
/// shape (e.g. `"lossless"`) — no client breakage.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Quality {
    Standard,
    Exhigh,
    #[default]
    Lossless,
    Hires,
    Sky,
    Jyeffect,
    Jymaster,
    Dolby,
}

impl Quality {
    /// All 8 variants in canonical wire-format order.
    /// Adding a new variant fails the `wire_str_round_trip` test below
    /// AND every exhaustive match site at compile time.
    pub const ALL: [Quality; 8] = [
        Quality::Standard,
        Quality::Exhigh,
        Quality::Lossless,
        Quality::Hires,
        Quality::Sky,
        Quality::Jyeffect,
        Quality::Jymaster,
        Quality::Dolby,
    ];

    pub fn wire_str(self) -> &'static str {
        match self {
            Quality::Standard => "standard",
            Quality::Exhigh => "exhigh",
            Quality::Lossless => "lossless",
            Quality::Hires => "hires",
            Quality::Sky => "sky",
            Quality::Jyeffect => "jyeffect",
            Quality::Jymaster => "jymaster",
            Quality::Dolby => "dolby",
        }
    }

    pub fn display_name_zh(self) -> &'static str {
        match self {
            Quality::Standard => "标准音质",
            Quality::Exhigh => "极高音质",
            Quality::Lossless => "无损音质",
            Quality::Hires => "Hi-Res音质",
            Quality::Sky => "沉浸环绕声",
            Quality::Jyeffect => "高清环绕声",
            Quality::Jymaster => "超清母带",
            Quality::Dolby => "杜比全景声",
        }
    }

    /// PR-B 降级阶梯：`Hires → Lossless → Exhigh → Standard`。
    /// premium 内容（Sky/Jyeffect/Jymaster/Dolby）**不**参与 fallback——
    /// 用户付费内容缺则缺，不应自动 silently 降到普通品质。
    ///
    /// 行为：
    /// - `start` 在 ORDER 中 → 从 `start` 索引到 `floor` 索引（含），最多 4 项
    /// - `start` 不在 ORDER 中（premium）→ 仅产出 `[start]`
    /// - `floor` 高于 `start`（用户配置错乱）→ 仅产出 `[start]`，避免无限或空 iter
    pub fn ladder(start: Quality, floor: Quality) -> impl Iterator<Item = Quality> {
        const ORDER: [Quality; 4] = [
            Quality::Hires,
            Quality::Lossless,
            Quality::Exhigh,
            Quality::Standard,
        ];
        let start_idx = ORDER.iter().position(|q| *q == start);
        let floor_idx = ORDER.iter().position(|q| *q == floor);
        match (start_idx, floor_idx) {
            (Some(s), Some(f)) if s <= f => ORDER[s..=f].to_vec(),
            (Some(_), None) => vec![start], // floor 是 premium → 不降级
            _ => vec![start],
        }
        .into_iter()
    }
}

impl fmt::Display for Quality {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.wire_str())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InvalidQuality(pub String);
impl fmt::Display for InvalidQuality {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "invalid quality: {}", self.0)
    }
}
impl std::error::Error for InvalidQuality {}

impl FromStr for Quality {
    type Err = InvalidQuality;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "standard" => Ok(Quality::Standard),
            "exhigh" => Ok(Quality::Exhigh),
            "lossless" => Ok(Quality::Lossless),
            "hires" => Ok(Quality::Hires),
            "sky" => Ok(Quality::Sky),
            "jyeffect" => Ok(Quality::Jyeffect),
            "jymaster" => Ok(Quality::Jymaster),
            "dolby" => Ok(Quality::Dolby),
            other => Err(InvalidQuality(other.into())),
        }
    }
}

// ===== Compat shims (PR-4 — derived from Quality enum) =====
// Migration path: PR-6 typed parsers will accept Quality at boundaries;
// PR-7 typestate will remove these shims after grep-clean.

/// Default wire-format quality string. Replaces 6 scattered
/// `unwrap_or_else(|| "lossless".into())` sites.
pub const DEFAULT_QUALITY: &str = "lossless";

/// Compat list — kept in lock-step with `Quality::ALL` via test below.
pub const VALID_QUALITIES: &[&str] = &[
    "standard", "exhigh", "lossless", "hires", "sky", "jyeffect", "jymaster", "dolby",
];

pub const VALID_TYPES: &[&str] = &["url", "name", "lyric", "json"];

/// Compat shim: legacy string-based display name lookup.
/// Routes through `Quality::FromStr` then `display_name_zh`.
pub fn quality_display_name(quality: &str) -> &'static str {
    Quality::from_str(quality)
        .map(|q| q.display_name_zh())
        .unwrap_or("未知音质")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_const_has_eight_variants() {
        assert_eq!(Quality::ALL.len(), 8);
    }

    #[test]
    fn wire_str_round_trip_all_variants() {
        for q in Quality::ALL {
            let s = q.wire_str();
            let parsed = Quality::from_str(s).expect("round-trip parse");
            assert_eq!(parsed, q, "wire_str → from_str must round-trip");
        }
    }

    #[test]
    fn default_is_lossless() {
        assert_eq!(Quality::default(), Quality::Lossless);
        assert_eq!(Quality::default().wire_str(), DEFAULT_QUALITY);
    }

    #[test]
    fn display_name_zh_for_each_variant() {
        // Direct test (test-gate satisfaction) — exhaustive coverage of all 8.
        assert_eq!(Quality::Standard.display_name_zh(), "标准音质");
        assert_eq!(Quality::Exhigh.display_name_zh(), "极高音质");
        assert_eq!(Quality::Lossless.display_name_zh(), "无损音质");
        assert_eq!(Quality::Hires.display_name_zh(), "Hi-Res音质");
        assert_eq!(Quality::Sky.display_name_zh(), "沉浸环绕声");
        assert_eq!(Quality::Jyeffect.display_name_zh(), "高清环绕声");
        assert_eq!(Quality::Jymaster.display_name_zh(), "超清母带");
        assert_eq!(Quality::Dolby.display_name_zh(), "杜比全景声");
    }

    #[test]
    fn serde_round_trip() {
        for q in Quality::ALL {
            let json = serde_json::to_string(&q).expect("serialize");
            let parsed: Quality = serde_json::from_str(&json).expect("deserialize");
            assert_eq!(parsed, q);
        }
    }

    #[test]
    fn serde_uses_lowercase_wire_format() {
        assert_eq!(
            serde_json::to_string(&Quality::Lossless).unwrap(),
            "\"lossless\""
        );
        assert_eq!(serde_json::to_string(&Quality::Dolby).unwrap(), "\"dolby\"");
    }

    #[test]
    fn from_str_rejects_unknown() {
        assert!(Quality::from_str("master").is_err());
        assert!(Quality::from_str("").is_err());
        assert!(Quality::from_str("LOSSLESS").is_err()); // case-sensitive
    }

    #[test]
    fn valid_qualities_const_in_lockstep_with_enum() {
        // PR-4 SOT invariant: VALID_QUALITIES must mirror Quality::ALL exactly.
        // Adding a variant without updating VALID_QUALITIES fails this test.
        let from_enum: Vec<&str> = Quality::ALL.iter().map(|q| q.wire_str()).collect();
        let from_const: Vec<&str> = VALID_QUALITIES.to_vec();
        assert_eq!(from_enum, from_const);
    }

    #[test]
    fn quality_display_name_compat_routes_through_enum() {
        assert_eq!(quality_display_name("lossless"), "无损音质");
        assert_eq!(quality_display_name("dolby"), "杜比全景声");
        assert_eq!(quality_display_name("garbage"), "未知音质");
    }

    #[test]
    fn display_trait_uses_wire_format() {
        assert_eq!(format!("{}", Quality::Lossless), "lossless");
    }

    #[test]
    fn invalid_quality_error_displays_input() {
        let err = Quality::from_str("foo").unwrap_err();
        assert_eq!(err.0, "foo");
        assert!(format!("{}", err).contains("foo"));
    }

    // ===== PR-B Quality::ladder =====

    #[test]
    fn ladder_terminates_at_floor() {
        // Hires → Lossless → Exhigh → Standard，floor=Standard 全 4 项
        let v: Vec<Quality> = Quality::ladder(Quality::Hires, Quality::Standard).collect();
        assert_eq!(
            v,
            vec![
                Quality::Hires,
                Quality::Lossless,
                Quality::Exhigh,
                Quality::Standard
            ]
        );
    }

    #[test]
    fn ladder_floor_lossless_stops_above_exhigh() {
        // Attacker：用户配置 floor=Lossless 不应降级到更低
        let v: Vec<Quality> = Quality::ladder(Quality::Hires, Quality::Lossless).collect();
        assert_eq!(v, vec![Quality::Hires, Quality::Lossless]);
    }

    #[test]
    fn ladder_skips_premium_qualities() {
        // Attacker：付费内容不参与 fallback，premium start 只产 [start] 不降级
        for premium in [
            Quality::Sky,
            Quality::Jyeffect,
            Quality::Jymaster,
            Quality::Dolby,
        ] {
            let v: Vec<Quality> = Quality::ladder(premium, Quality::Standard).collect();
            assert_eq!(v, vec![premium], "premium {:?} 不应降级", premium);
        }
    }

    #[test]
    fn ladder_floor_above_start_returns_just_start() {
        // 用户配置错乱：start=Standard, floor=Hires → 不应无限循环或空 iter
        let v: Vec<Quality> = Quality::ladder(Quality::Standard, Quality::Hires).collect();
        assert_eq!(v, vec![Quality::Standard]);
    }

    #[test]
    fn ladder_start_equals_floor_returns_one() {
        let v: Vec<Quality> = Quality::ladder(Quality::Lossless, Quality::Lossless).collect();
        assert_eq!(v, vec![Quality::Lossless]);
    }

    #[test]
    fn ladder_floor_premium_treats_as_no_descend() {
        // floor 是 premium → 等同 floor 找不到，仅产 start
        let v: Vec<Quality> = Quality::ladder(Quality::Lossless, Quality::Dolby).collect();
        assert_eq!(v, vec![Quality::Lossless]);
    }
}
