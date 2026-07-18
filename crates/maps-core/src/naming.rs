//! Map titles from a small Tracery-style grammar. Water presence biases the
//! vocabulary toward damp names, mirroring the original; forest mode swaps
//! in a woodland lexicon.

use crate::Mode;
use rand::Rng;

struct Lexicon {
    adj: &'static [&'static str],
    noun: &'static [&'static str],
    wet_noun: &'static [&'static str],
    owner: &'static [&'static str],
    pre: &'static [&'static str],
    suf: &'static [&'static str],
}

const CAVE: Lexicon = Lexicon {
    adj: &[
        "Whispering", "Sunless", "Forgotten", "Howling", "Shattered", "Silent", "Weeping",
        "Black", "Broken", "Hidden", "Ancient", "Crimson", "Endless", "Pale", "Gnawed",
        "Crooked", "Drowned", "Trembling", "Starved", "Echoing",
    ],
    noun: &[
        "Cave", "Caves", "Cavern", "Caverns", "Grotto", "Hollow", "Hollows", "Warren", "Den",
        "Lair", "Burrow", "Depths", "Chasm", "Catacombs", "Tunnels", "Pit", "Maw", "Delve",
        "Gallery", "Rift",
    ],
    wet_noun: &[
        "Bog", "Mire", "Sump", "Cistern", "Pools", "Fen", "Wellspring", "Sinkhole",
    ],
    owner: &[
        "the Wyrm", "the Deep King", "Echoes", "Sorrows", "the Pale Moon", "Embers",
        "the Drowned", "the First Ones", "the Silent Horde", "Teeth", "the Old Dark", "Regret",
        "the Undermarch",
    ],
    pre: &[
        "Wyrm", "Bone", "Salt", "Ember", "Black", "Deep", "Mud", "Frost", "Shadow", "Rot",
    ],
    suf: &[
        "hollow", "maw", "delve", "warren", "gullet", "fen", "deep", "burrow", "gnaw",
    ],
};

const FOREST: Lexicon = Lexicon {
    adj: &[
        "Whispering", "Mossy", "Tangled", "Silver", "Elder", "Sleeping", "Sunlit", "Quiet",
        "Green", "Wild", "Fey", "Hidden", "Ancient", "Crooked", "Shivering", "Bramble",
    ],
    noun: &[
        "Glade", "Glades", "Clearing", "Grove", "Dell", "Meadow", "Thicket", "Copse", "Bower",
        "Heath", "Weald", "Hollow",
    ],
    wet_noun: &["Fen", "Mire", "Marsh", "Pools", "Bog", "Spring"],
    owner: &[
        "the Elk King", "the Green Lady", "the Fey", "Antlers", "the Old Oak", "Nightingales",
        "the Wardens", "Moss", "the Pale Moon", "Foxes", "the Quiet Folk",
    ],
    pre: &[
        "Oak", "Moss", "Fern", "Bramble", "Thorn", "Willow", "Deer", "Fox", "Birch", "Hazel",
    ],
    suf: &["glade", "grove", "dell", "mead", "brake", "shade", "hollow"],
};

const NAME: &[&str] = &[
    "Torvak", "Mabreth", "Uln", "Skarn", "Vethra", "Old Hem", "Karsu", "the Hermit", "Grelda",
];

fn pick<'a, R: Rng>(rng: &mut R, list: &[&'a str]) -> &'a str {
    list[rng.random_range(0..list.len())]
}

pub fn title<R: Rng>(rng: &mut R, wet: bool, mode: Mode) -> String {
    let lex = match mode {
        Mode::Cave => &CAVE,
        Mode::Forest => &FOREST,
    };
    let noun = if wet && rng.random_bool(0.55) {
        pick(rng, lex.wet_noun)
    } else {
        pick(rng, lex.noun)
    };
    match rng.random_range(0..6) {
        0 | 1 => format!("The {} {}", pick(rng, lex.adj), noun),
        2 => format!("The {} of {}", noun, pick(rng, lex.owner)),
        3 => format!("{}'s {}", pick(rng, NAME), noun),
        4 => format!("{}{} {}", pick(rng, lex.pre), pick(rng, lex.suf), noun),
        _ => format!("The {} {} of {}", pick(rng, lex.adj), noun, pick(rng, lex.owner)),
    }
}
