//! Fonts that can be embedded with Ghostty. Note they are only actually
//! embedded in the binary if they are referenced by the code, so fonts
//! used for tests will not result in the final binary being larger.
//!
//! Be careful to ensure that any fonts you embed are licensed for
//! redistribution and include their license as necessary (see `res/`).

/// Default fonts that we prefer for Ghostty.
pub static VARIABLE: &[u8] = include_bytes!("../res/JetBrainsMono-Variable.ttf");
pub static VARIABLE_ITALIC: &[u8] = include_bytes!("../res/JetBrainsMono-Italic-Variable.ttf");

/// Symbols-only nerd font.
pub static SYMBOLS_NERD_FONT: &[u8] = include_bytes!("../res/SymbolsNerdFont-Regular.ttf");

/// Static jetbrains mono faces, currently unused (except by tests).
pub static REGULAR: &[u8] = include_bytes!("../res/JetBrainsMono-Regular.ttf");
pub static BOLD: &[u8] = include_bytes!("../res/JetBrainsMono-Bold.ttf");
pub static ITALIC: &[u8] = include_bytes!("../res/JetBrainsMono-Italic.ttf");
pub static BOLD_ITALIC: &[u8] = include_bytes!("../res/JetBrainsMono-BoldItalic.ttf");

/// Emoji fonts
pub static EMOJI: &[u8] = include_bytes!("../res/NotoColorEmoji.ttf");
pub static EMOJI_TEXT: &[u8] = include_bytes!("../res/NotoEmoji-Regular.ttf");

// Fonts below are ONLY used for testing.

/// Fonts with general properties
pub static ARABIC: &[u8] = include_bytes!("../res/KawkabMono-Regular.ttf");

/// A font for testing which is patched with nerd font symbols.
pub static TEST_NERD_FONT: &[u8] = include_bytes!("../res/JetBrainsMonoNerdFont-Regular.ttf");

/// Specific font families below:
pub static CODE_NEW_ROMAN: &[u8] = include_bytes!("../res/CodeNewRoman-Regular.otf");
pub static INCONSOLATA: &[u8] = include_bytes!("../res/Inconsolata-Regular.ttf");
pub static GEIST_MONO: &[u8] = include_bytes!("../res/GeistMono-Regular.ttf");
pub static JETBRAINS_MONO: &[u8] = include_bytes!("../res/JetBrainsMonoNoNF-Regular.ttf");
pub static JULIA_MONO: &[u8] = include_bytes!("../res/JuliaMono-Regular.ttf");

/// Cozette is a unique font because it embeds some emoji characters
/// but has a text presentation.
pub static COZETTE: &[u8] = include_bytes!("../res/CozetteVector.ttf");

/// Monaspace has weird ligature behaviors we want to test in our shapers
/// so we embed it here.
pub static MONASPACE_NEON: &[u8] = include_bytes!("../res/MonaspaceNeon-Regular.otf");

/// Terminus TTF is a scalable font with bitmap glyphs at various sizes.
pub static TERMINUS_TTF: &[u8] = include_bytes!("../res/TerminusTTF-Regular.ttf");

/// Spleen is a monospaced bitmap font available in multiple formats.
/// Used for testing bitmap font support across different file formats.
pub static SPLEEN_BDF: &[u8] = include_bytes!("../res/spleen-8x16.bdf");
pub static SPLEEN_PCF: &[u8] = include_bytes!("../res/spleen-8x16.pcf");
pub static SPLEEN_OTB: &[u8] = include_bytes!("../res/spleen-8x16.otb");
