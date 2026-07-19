use crate::contracts::RuntimeUiPreferences;
use crate::protocol::RuntimeUiPreferencesV3;

pub(crate) fn parse(preferences: RuntimeUiPreferencesV3) -> Result<RuntimeUiPreferences, String> {
    if !is_valid_theme_preference(&preferences.theme) {
        return Err("runtime_ui_theme_invalid".to_string());
    }
    if !is_valid_history_point_limit(preferences.history_point_limit) {
        return Err("runtime_history_point_limit_invalid".to_string());
    }
    Ok(RuntimeUiPreferences {
        theme: preferences.theme,
        history_point_limit: preferences.history_point_limit,
    })
}

pub(crate) fn is_valid(preferences: &RuntimeUiPreferences) -> bool {
    is_valid_theme_preference(&preferences.theme)
        && is_valid_history_point_limit(preferences.history_point_limit)
}

fn is_valid_theme_preference(value: &str) -> bool {
    if matches!(value, "system" | "cave" | "aurora" | "ember" | "daylight") {
        return true;
    }

    let Some((family, mode)) = value.split_once(':') else {
        return false;
    };
    matches!(family, "cave" | "aurora" | "ember" | "canopy")
        && matches!(mode, "system" | "light" | "dark")
}

fn is_valid_history_point_limit(value: u32) -> bool {
    matches!(value, 30 | 72 | 180 | 360)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_every_paired_theme_preference() {
        for family in ["cave", "aurora", "ember", "canopy"] {
            for mode in ["system", "light", "dark"] {
                let theme = format!("{family}:{mode}");
                let parsed = parse(RuntimeUiPreferencesV3 {
                    theme: theme.clone(),
                    history_point_limit: 180,
                })
                .expect("paired preference converts");
                assert_eq!(parsed.theme, theme);
            }
        }
    }

    #[test]
    fn accepts_legacy_themes_without_normalizing_before_a_durable_write() {
        for theme in ["system", "cave", "aurora", "ember", "daylight"] {
            let parsed = parse(RuntimeUiPreferencesV3 {
                theme: theme.to_string(),
                history_point_limit: 72,
            })
            .expect("legacy preference remains readable");
            assert_eq!(parsed.theme, theme);
        }
    }

    #[test]
    fn rejects_invalid_theme_combinations() {
        for theme in [
            "",
            "auto",
            "canopy",
            "daylight:light",
            "cave:auto",
            "cave:system:dark",
            "Cave:dark",
            ":dark",
            "cave:",
        ] {
            assert_eq!(
                parse(RuntimeUiPreferencesV3 {
                    theme: theme.to_string(),
                    history_point_limit: 180,
                }),
                Err("runtime_ui_theme_invalid".to_string()),
                "{theme}"
            );
        }
    }

    #[test]
    fn rejects_invalid_history_point_limits() {
        assert_eq!(
            parse(RuntimeUiPreferencesV3 {
                theme: "canopy:system".to_string(),
                history_point_limit: 10_000,
            }),
            Err("runtime_history_point_limit_invalid".to_string())
        );
    }

    #[test]
    fn validates_persisted_preferences_with_the_same_contract() {
        assert!(is_valid(&RuntimeUiPreferences {
            theme: "aurora:light".to_string(),
            history_point_limit: 30,
        }));
        assert!(!is_valid(&RuntimeUiPreferences {
            theme: "aurora:auto".to_string(),
            history_point_limit: 30,
        }));
        assert!(!is_valid(&RuntimeUiPreferences {
            theme: "aurora:light".to_string(),
            history_point_limit: 31,
        }));
    }
}
