//! Provides unit tests easy ways to generate compilable source files from statements, when they
//! don't really care that much about the metadata.

#![allow(unused)]

use super::Format;
use truth::Game;

pub const ANM_06: Format = Format {
    cmd: "truanm",
    game: Game::Th06,
    script_head: r#"
#pragma mapfile "map/any.anmm"

entry {
    path: "subdir/file.png",
    has_data: false,
    img_width: 512,
    img_height: 512,
    img_format: 3,
    offset_x: 0,
    offset_y: 0,
    colorkey: 0,
    memory_priority: 0,
    low_res_scale: false,
    sprites: {
        sprite0: {id: 0, x: 0.0, y: 0.0, w: 512.0, h: 480.0},
        sprite1: {id: 1, x: 0.0, y: 0.0, w: 512.0, h: 480.0},
        sprite2: {id: 2, x: 0.0, y: 0.0, w: 512.0, h: 480.0},
        sprite3: {id: 3, x: 0.0, y: 0.0, w: 512.0, h: 480.0},
        sprite4: {id: 4, x: 0.0, y: 0.0, w: 512.0, h: 480.0},
    },
}
"#,
    make_main: |body| format!(r#"
script script0 {{
    {}
}}
"#, body),
};

pub const ANM_10: Format = Format {
    cmd: "truanm",
    game: Game::Th10,
    script_head: ANM_06.script_head,
    make_main: ANM_06.make_main,
};

pub const ANM_12: Format = Format {
    cmd: "truanm",
    game: Game::Th12,
    script_head: ANM_06.script_head,
    make_main: ANM_06.make_main,
};

pub const ANM_16: Format = Format {
    cmd: "truanm",
    game: Game::Th16,
    script_head: ANM_06.script_head,
    make_main: ANM_06.make_main,
};

pub const STD_06: Format = Format {
    cmd: "trustd",
    game: Game::Th06,
    script_head: r#"
#pragma mapfile "map/any.stdm"

meta {
    unknown: 0,
    stage_name: "dm",
    bgm: [
        {path: "bgm/th08_08.mid", name: "dm"},
        {path: "bgm/th08_09.mid", name: "dm"},
        {path: " ", name: " "},
        {path: " ", name: " "},
    ],
    objects: {},
    instances: [],
}
"#,
    make_main: |body| format!(r#"
script main {{
    {}
}}
"#, body),
};

pub const STD_08: Format = Format {
    cmd: "trustd",
    game: Game::Th08,
    script_head: STD_06.script_head,
    make_main: STD_06.make_main,
};

pub const STD_12: Format = Format {
    cmd: "trustd",
    game: Game::Th12,
    script_head: r#"
#pragma mapfile "map/any.stdm"

meta {
    unknown: 0,
    anm_path: "stage01.anm",
    objects: {
        thing: {
            layer: 4,
            pos: [10.0, 20.0, 30.0],
            size: [10.0, 20.0, 30.0],
            quads: [],
        },
    },
    instances: [],
}
"#,
    make_main: STD_06.make_main,
};

pub const MSG_06: Format = Format {
    cmd: "trumsg",
    game: Game::Th06,
    script_head: r#"
#pragma mapfile "map/any.msgm"

meta {
    table: {
        0: {script: "script0"},
    }
}
"#,
    make_main: |body| format!(r#"
script main {{
    {}
}}
"#, body),
};

pub const MSG_08: Format = Format {
    cmd: "trumsg",
    game: Game::Th08,
    script_head: MSG_06.script_head,
    make_main: MSG_06.make_main,
};

pub const MSG_09: Format = Format {
    cmd: "trumsg",
    game: Game::Th09,
    script_head:  r#"
#pragma mapfile "map/any.msgm"

meta {
    table: {
        0: {script: "script0", flags: 256},
    }
}
"#,
    make_main: MSG_06.make_main,
};

pub const MSG_11: Format = Format {
    cmd: "trumsg",
    game: Game::Th11,
    script_head: MSG_09.script_head,
    make_main: MSG_09.make_main,
};

pub const MSG_12: Format = Format {
    cmd: "trumsg",
    game: Game::Th12,
    script_head: MSG_09.script_head,
    make_main: MSG_09.make_main,
};

pub const MSG_17: Format = Format {
    cmd: "trumsg",
    game: Game::Th17,
    script_head: MSG_09.script_head,
    make_main: MSG_09.make_main,
};

pub const ECL_06: Format = Format {
    cmd: "truecl",
    game: Game::Th06,
    script_head: r#"
#pragma mapfile "map/any.eclm"

timeline 0 {}
"#,
    make_main: |body| format!(r#"
void sub0() {{
    {}
}}
"#, body),
};

// FIXME: Maybe default difficulty names shouldn't be in the default mapfile
pub const ECL_06_NO_DEFAULT_MAP: Format = Format {
    cmd: "truecl",
    game: Game::Th06,
    script_head: r#"
timeline 0 {}
"#,
    make_main: |body| format!(r#"
void sub0() {{
    {}
}}
"#, body),
};

pub const ECL_07: Format = Format {
    game: Game::Th07,
    ..ECL_06
};

pub const ECL_08: Format = Format {
    game: Game::Th08,
    ..ECL_06
};

/// Variant of ECL_06 where main_body is inserted into a timeline instead.
/// There are no subs by default.
pub const ECL_TIMELINE_06: Format = Format {
    cmd: "truecl",
    game: Game::Th06,
    script_head: r#""#,
    make_main: |body| format!(r#"
timeline 0 {{
    {}
}}
"#, body),
};

pub const ECL_TIMELINE_08: Format = Format {
    cmd: "truecl",
    game: Game::Th08,
    script_head: r#""#,
    make_main: |body| format!(r#"
timeline 0 {{
    {}
}}
"#, body),
};

