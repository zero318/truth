# Unreleased

## Added

* **`truecl`** (!!!!) is available in **prototype status,** but the way to invoke it is ~~a well guarded secret~~ `truth-core truecl`.  TH06-TH095 are supported.
* **Function definition syntax for exported subs.**  `void Sub0(int x) {}`
* **Natural call syntax for exported subs.**  `Sub0(10, 20.4);`
* **Difficulty switches.**  `I0 = A + (3:4:4:5);`
* **Difficulty flags.** `{"ENH"}: ins_10();`
* **`@arg0` pseudo-arg.**  This will be used together with `@blob` when decompiling timelines with unknown signatures in TH06 and TH07.
* **Difficulty flag names.** (`!difficulty_flags`)  The prepackaged maps do this.
* **Debug info.** The `--output-debug-info JSONFILE` flag has been added to all `compile` subcommands.  This file can be used by debuggers to relate source-level concepts (e.g. function names and variable names) to runtime concepts (e.g. registers and const values).

  Perfect name resolution of locals in a debugger might not be possible yet as truth currently does not explicitly construct the tree used for name resolution (it is only implicitly constructed), so scope information is not yet available.
* **Bitwise negation.** The unary `~` (bitwise not) operator has been added.  No language natively provides it, but even when unavailable, it can be used (and will compile as `-1 - x`).
* **`--no-builtin-mapfiles`** option for both compilation and decompilation.  This will disable the core mapfiles which provide the builtin signatures and intrinsic mappings for all games, which can be useful if you are trying to design a comprehensive mapfile from scratch.

## Other bugfixes

* Unary `-` now can be directly provided by an intrinsic, and will otherwise fall back to `-1 * x`. (formerly, it would always produce `0 - x`, which is not correct for all floats)

# Version 0.5.0

Version 0.5.0 is a major release of the compiler containing many of the language additions that were found useful in alpha implementations of truecl.  While `truecl` is still not officially released, these additions and improvements are available in all languages.

## Added

### Commands

* Multiple `-m` args can now be provided.
* Decompile commands now support `-o`/`--output`, just like compilation.  This is extremely useful on Windows where redirection on STDOUT may produce a file in a different encoding from UTF-8 depending on the system configuration.
* truanm had some unhelpful behavior when using multiple image sources that provide the same image, or when using `has_data: "dummy"` together with an image source.  Image sources have been redesigned to better support common use cases.

### Language features

* **`INF`, `NAN`, `PI` constants.**
* **`break` keyword.**  This exits the nearest surrounding loop.
* **New type-cast syntax.**  The old `_S` and `_f` functions have been split into two types of operations: `int(expr)` and `float(expr)` for type-casts, and `$(expr)` and `%(expr)` to read a temporary as some type.  (the two are the same in most languages, except EoSD ECL which does not auto-cast)
* **`offsetof(label)` and `timeof(label)` expressions.**  You can use these if you want to write a jump using `ins_` syntax or the instruction alias. (these were a necessary addition for decompilation of EoSD ECL files that use conditional jumps in a funny way).
* **User-defined enums.**  A mapfile can define an enum:
  ```
  !enum(name="TestEnum")
  20 Red
  40 Blue
  ```
  which defines constants `TestEnum.Red` and `TestEnum.Blue`.  An instruction can then specify that it takes an argument of this type, via a modified `S` argument:
  ```
  !ins_signatures
  100 S(enum="TestEnum")
  ```
  In this example, when calling `ins_100` you will be able to use `ins_100(Red)` as a shorthand for `ins_100(TestEnum.Red)`, and during decompilation it will try to decompile the values from this enum. Enums are *open,* in that the same enum can be defined multiple times or even across multiple mapfiles, and the list of consts will be merged.
* **Built-in enums:**
  * **`bool`.** `true` and `false` are no longer keywords but rather members of a builtin `enum bool`.  Use it like any other enum.
  * **`BitmapColorFormat`.**  This now houses the constants `FORMAT_ARGB_8888`, `FORMAT_RGB_565`, `FORMAT_ARGB_4444`, and `FORMAT_GRAY_8` used by `img_format` and `rt_format` in ANM files.
  * **`AnmSprite`, `AnmScript`, `MsgScript`, `EclSub`.** These get automatically generated from the names defined in a source script, allowing you to write e.g. `S(enum="AnmSprite")` and `S(enum="AnmScript")` instead of `n` and `N`.  This makes them more flexible, allowing these types to be used in word-sized arguments and, in the future, timeline arg0. 
* **Intrinsic mappings.** (`!ins_intrinsics`)  For instance, a patch which adds jump intrinsics to MSG could also provide a mapfile which tells trumsg about this intrinsic:
  ```
  !msgmap
  !ins_instrinsics
  100 Jmp()
  !ins_signatures
  100 ot
  ```
  and then you would be able to write `loop { }`s in MSG!

### Improvements to decompilation

* Can now detect of `if/elseif` chains that have no `else` block.
* Many improvements to detection of nested `if/else`s and loops.
* Decompiling intrinsics will fall back to instruction syntax if the intrinsic cannot be decompiled. (e.g. PCB stage 7 ECL has a `set_int` instruction that tries to assign to an immediate)
* Decompiling sub/script/sprite names will fall back to raw integers if the corresponding items don't exist.

## Compatibility notes

* Using registers (e.g. `$REG[8]`) in a format without registers such as STD is now detected as an error.
* If e.g. `X` is an alias for `$REG[3]`, then using both `X` and `REG[3]` in the same function body will now generate a warning.  Similarly, using two different aliases (e.g. `X` and `Y`) for the same register will also warn.
  * This is partly so that, in future versions of truth that provide `truecl`, the compiler can call attention to accidental usage of EoSD's `I0` or PCB's "param" registers in subs where these registers are already implicitly in use by function parameters.
* Previously, using `-m mapfile.eclm` during decompilation would disable lookup from `TRUTH_MAP_PATH`.  Now that multiple `-m` are supported, this behavior now seems surprising, so `TRUTH_MAP_PATH` is now *always* searched during decompilation.

## Internal changes

* The order and encoding of arguments to intrinsics is no longer hardcoded by game/format, but rather inferred from the mapfile signature, meaning it can be defined by the user.  So for instance, a `CondJmp(op="=="; type="float")` could have any of the signatures `otff`, `toff`, `ffto`, or `ffot`, and these would all be encoded correctly by the compiler.
* Time labels are now internally stored as statements.  This drastically simplifies parsing and some aspects of loop compilation/decompilation.

# Version 0.4.3

0.4.3 is a minor update that adds some information from Dai:

* The `unknown` field in STD objects has been renamed to `layer`; it behaves like the `layer(_)` command, and is used to control the layering of 3D objects relative to 2D sprites.  The old name is still accepted, but may be deprecated and eventually removed in the distant future.
* Small update to mapfiles.

# Version 0.4.2

## Added

* Adds `truanm extract` (`truanm x`) to extract images.  See the [readme](https://github.com/ExpHP/truth#readme) for more details.
* Adds additional flags to tweak decompilation. These can help work with unusual files.
    * `--no-blocks`: Disables conditional and loop decompilation, leaving behind all `goto`s and labels in their natural state.
    * `--no-intrinsics`: Forces every instruction to decompile into `name(arg1, arg2)` etc.  (e.g. `addf(F1, 0.3)` instead of `F1 += 0.3;`).
    * `--no-arguments`: Forces all instructions to decompile into raw form using pseudo-arguments. (e.g. `addi(@mask=0b1, @blob="10270000 24000000")`).
* Bugfix:  Floating point comparisons actually work now instead of causing a panic.  So that's cool.

# Version 0.4.1

## Added

* **Support for TH18: 東方虹龍洞 ～ Unconnected Marketeers has been added.**
* `truanm` can now import image files (PNG, BMP, GIF).  Simply supply a directory instead of an ANM file to `--image-source` or `#pragma image_source`.
* `truanm` can generate dummy image data to be hotswapped by thcrap; try `has_data: "generate"`.
* `truanm` now has named constants for color formats: `FORMAT_ARGB_8888, FORMAT_ARGB_4444, FORMAT_RGB_565, FORMAT_GRAY_8`.
* `truanm` now provides sane defaults for the majority of fields on an `entry`.

## Compatibility notes

* Renamed some ANM entry fields to place greater focus on the things you should probably care more about:
    * `width`, `height`, and `format` have been renamed to `rt_width, rt_height, rt_format` to indicate that these are **values for the runtime buffer.** Generally speaking, you should almost never need to worry about these.
    * `thtx_width, thtx_height, thtx_format` have been renamed to `img_width, img_height, img_format` to make them less "scary."  These describe the actual dimensions and format of an embedded image.
    * The old names still work, but will generate a deprecation warning.

# Version 0.4.0

## Added

* **`trumsg`** is released!!
* `trumsg` comes fully tested on all (stage) MSG files in all games up to and including TH17.
* Added **pseudo-arguments.** When decompiling, instructions with unknown signatures will now be decoded to a form like `ins_1011(@mask=0b100, @blob="00501c46 00000040 00541c46");`
* Added **`const` variables.** These are typed compile-time constants.  Please see the [truth syntax primer](https://github.com/ExpHP/truth/blob/main/doc/syntax.md) for more information.
* **Warnings!**  Diagnostics were heavily reworked, allowing warnings to be emitted for many more things (and allowing some things that used to be errors to become warnings instead).

## Compatibility notes

* Text source files are now [**always** UTF-8, **never** Shift-JIS](https://github.com/ExpHP/truth/issues/2#issuecomment-770449571).  If you were previously using `truth` with Shift-JIS files, you may need to change their encoding and re-save them in your text editor.
* Default decompilation output line width was changed from 100 to 80.

# Version 0.0.3

* Added word-sized arguments.
* Big cleanup of how type checking and name resolution works, with generally better error messages for many things.
* Sprite names and script names now can be used as expressions (e.g. `scriptNew(script20)`), and such names will be produced while decompiling as well.
* Added a `--output-thecl-defs FILENAME` flag to `truanm` that works like thanm's `-s`, as this may have been a blocker for adoption by some.  This may be a somewhat temporary solution, as the goal is for `truecl` to have its own way of importing an anm script directly.

## Compatibility notes

* ANM's `anchor` now takes two words in some games.

# Version 0.0.2 (Jan 25, 2021)

This is the first release made available online.  It contains:

* A fully functional `trustd`
* A largely functional `truanm`.  It can decompile and recompile all vanilla ANM files.  However, not all language features are implemented yet.
