//! 官方索引里的引导推荐清单类型。

use super::super::manifest::LocalizedText;

/// 官方索引里的引导推荐清单（ADR-0034）。
#[derive(Debug, Clone, PartialEq, Eq, serde::Deserialize)]
pub struct OnboardingCatalog {
    pub agents: Vec<String>,
    pub needs: Vec<OnboardingNeed>,
    pub prompts: Vec<LocalizedText>,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Deserialize)]
pub struct OnboardingNeed {
    pub id: String,
    pub title: LocalizedText,
    pub plugins: Vec<String>,
}
