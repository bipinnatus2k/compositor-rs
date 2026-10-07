//! Port of the Levels sheet (`LevelsSheet.swift`). The sheet body lands
//! here as it is carved out of sheets_ui.rs; today this holds the shared
//! Levels UI helpers.

use compositor_core::adjust::LevelsChannel;

/// The Levels/Curves channel picker's labels (LevelsSheet.swift's Picker).
pub fn levels_channel_label(channel: LevelsChannel) -> &'static str {
    match channel {
        LevelsChannel::Rgb => "RGB",
        LevelsChannel::Red => "Red",
        LevelsChannel::Green => "Green",
        LevelsChannel::Blue => "Blue",
    }
}

#[cfg(test)]
mod tests {
    use super::levels_channel_label;
    use compositor_core::adjust::LevelsChannel;

    #[test]
    fn channel_labels_match_the_swift_picker() {
        assert_eq!(levels_channel_label(LevelsChannel::Rgb), "RGB");
        assert_eq!(levels_channel_label(LevelsChannel::Red), "Red");
        assert_eq!(levels_channel_label(LevelsChannel::Green), "Green");
        assert_eq!(levels_channel_label(LevelsChannel::Blue), "Blue");
    }
}
