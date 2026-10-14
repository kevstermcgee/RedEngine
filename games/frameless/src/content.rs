//! A museum's collection (`museum.collection` of a scene: themes, words, art styles, masterpieces). Checker: `museum::spec::parse_collection`.

use red_engine2::content::common::Any;
use red_engine2::strict_struct;
use std::collections::BTreeMap;

strict_struct! {
    /// What a museum is made of.
    pub struct Collection {
        /// Art style names, in the order the game's `museum.art` shader numbers them.
        styles: Vec<String>,
        /// Sculpture material names, in the shader's order.
        materials: Vec<String>,
        /// Shared word lists: name -> words (a word may hold `{slots}`).
        words: BTreeMap<String, Vec<String>>,
        /// Artist-name templates: the label, then after ` // ` how a story names them.
        artists: Vec<String>,
        media: CollectionMedia,
        /// Origin stories any theme's paintings may be given.
        stories: Vec<String>,
        /// Origin stories for sculptures, installations and tapestries.
        object_stories: Vec<String>,
        themes: Vec<CollectionTheme>,
        masterpieces: Vec<Masterpiece>,
    }
}

strict_struct! {
    /// Media per kind of piece.
    pub struct CollectionMedia { painting: Vec<String>, tapestry: Vec<String>, sculpture: Vec<String>, installation: Vec<String> }
}

strict_struct! {
    /// A wing's mood.
    pub struct CollectionTheme {
        id: String,
        name: String,
        wing: String,
        colors: ThemeColors,
        /// Four-colour palettes, darkest first.
        palettes: Vec<Vec<String>>,
        styles: Vec<String>,
        sculptures: Vec<String>,
        /// Indices into `audio.journey.scores`.
        music: Vec<Any>,
        words: BTreeMap<String, Vec<String>>,
        titles: Vec<String>,
        stories: Vec<String>,
        hints: Vec<String>,
    }
}

strict_struct! {
    /// A theme's room colours.
    pub struct ThemeColors { wall: String, floor: String, trim: String, ceiling: String, light: String }
}

strict_struct! {
    /// A hand-written piece.
    pub struct Masterpiece { title: String, artist: String, year: String, medium: String, story: String, theme: String, style: String }
}
