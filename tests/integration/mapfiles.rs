#[allow(unused)]
use crate::integration_impl::{expected, formats::*};

source_test!(
    ANM_10, mapfile_does_not_exist,
    items: r#"
        #pragma mapfile "this/is/a/bad/path"
    "#,
    expect_error: "while resolving",
);

source_test!(
    ANM_10, seqmap_missing_section_header,
    mapfile: r#"!anmmap
300 ot  //~ ERROR missing section header
"#,
    main_body: r#""#,
);

source_test!(
    ANM_10, seqmap_missing_magic,
    mapfile: r#"//~ ERROR missing magic
300 ot
"#,
    main_body: r#""#,
);

source_test!(
    ANM_10, abi_multiple_o,
    mapfile: r#"!anmmap
!ins_signatures
300 oot   //~ ERROR multiple 'o'
"#,
    main_body: r#""#,
);

source_test!(
    ANM_10, seqmap_duplicate_key,
    mapfile: r#"!anmmap
!ins_signatures
99 S
!ins_names
99 blue
99 bloo
"#,
    main_body: r#"
    blue(5);
    bloo(7);
    "#,
    check_decompiled: |decompiled| {
        // prefer the most recent name
        assert!(decompiled.contains("bloo"));
        assert!(!decompiled.contains("blue"));
    },
);

source_test!(
    ANM_10, seqmap_duplicate_section,
    mapfile: r#"!anmmap
!ins_names
99 blue
!ins_signatures
99 S
!ins_names
99 bloo
"#,
    main_body: r#"
    blue(5);
    bloo(7);
    "#,
    check_compiled: |_, _| {
        // just need it to succeed...
    },
);

source_test!(
    ANM_10, keywords_or_forbidden_idents,
    mapfile: r#"!anmmap
!ins_names
99 break  //~ ERROR identifier
100 ins_200  //~ ERROR identifier
"#,
    main_body: "",
);

source_test!(
    ANM_10, intrinsic_name_garbage,
    mapfile: r#"!anmmap
!ins_intrinsics

# no parens
4 lmfao            //~ ERROR expected open paren

# xkcd 859
5 CondJmp(op=">";type="int"    //~ ERROR missing closing paren

# extra arg
6 Jmp(type="int")         //~ WARNING unknown attribute

# missing arg
7 CondJmp(op=">=")      //~ ERROR missing attribute

# intrinsic name typo
8 CondimentJmp(op=">=";type="int")  //~ ERROR variant not found

# garbage after
9 CondJmp(op=">=";type="int") lol   //~ ERROR unexpected token

# integer
10 10()   //~ ERROR identifier

# keyword
11 break()   //~ ERROR identifier
"#,
    main_body: r#""#,
);

source_test!(
    ANM_10, intrinsic_jmp_needs_offset,
    mapfile: r#"!anmmap
!ins_intrinsics
4 Jmp()
!ins_signatures
4 St     //~ ERROR without an 'o'
"#,
    main_body: r#""#,
);

source_test!(
    ANM_10, intrinsic_jmp_mislabeled_time,
    mapfile: r#"!anmmap
!ins_intrinsics
4 Jmp()   //~ ERROR unexpected dword
!ins_signatures
4 So
"#,
    main_body: r#""#,
);

source_test!(
    ANM_10, intrinsic_jmp_non_consecutive_offset_time,
    mapfile: r#"!anmmap
!ins_signatures
300 tSo   //~ ERROR must be consecutive
!ins_intrinsics
300 CountJmp()
"#,
    main_body: r#""#,
);

source_test!(
    ANM_10, intrinsic_op_output_arg_wrong_type,
    mapfile: r#"!anmmap
!ins_intrinsics
99 AssignOp(op="="; type="int")  //~ ERROR unexpected encoding
!ins_signatures
99 fS
"#,
    main_body: r#""#,
);

source_test!(
    ANM_10, intrinsic_op_input_arg_wrong_type,
    mapfile: r#"!anmmap
!ins_intrinsics
99 AssignOp(op="="; type="int")  //~ ERROR unexpected encoding
!ins_signatures
99 Sf
"#,
    main_body: r#""#,
);

source_test!(
    ANM_10, intrinsic_has_extra_arg,
    mapfile: r#"!anmmap
!ins_intrinsics
99 AssignOp(op="="; type="int")   //~ ERROR unexpected
!ins_signatures
99 SSS
"#,
    main_body: r#""#,
);

source_test!(
    ANM_10, intrinsic_has_insufficient_args,
    mapfile: r#"!anmmap
!ins_intrinsics
99 AssignOp(op="="; type="int")  //~ ERROR not enough arguments
!ins_signatures
99 S
"#,
    main_body: r#""#,
);

source_test!(
    ANM_10, intrinsic_with_mismatched_signature_in_core_map,
    mapfile: r#"!anmmap
!ins_intrinsics
3 Jmp()    # id of 'sprite'  //~ ERROR missing jump offset
"#,
    main_body: r#""#,
);

source_test!(
    ANM_10, intrinsic_without_signature,
    mapfile: r#"!anmmap
!ins_intrinsics
999 Jmp()  //~ ERROR has no signature
"#,
    items: r#"
// this particular error used to be generated once per script (commit c051299ba21de11e),
// so put a couple here as a regression test.
// (they don't need to actually use the instruction)
script aaa { }
script bbb { }
"#,
    main_body: r#""#,
);

source_test!(
    ANM_10, intrinsic_for_op_that_no_game_has,
    mapfile: r#"!anmmap
!ins_intrinsics
999 BinOp(op=">>>"; type="int")
998 UnOp(op="~"; type="int")

!ins_signatures
999 SSS
998 SS
"#,
    main_body: r#"
    int x = 10;
    int y = 15;
    int z = x >>> (y + 3);
    int w = ~x;
"#,
    check_compiled: |output, format| {
        let ecl = output.read_anm(format);
        assert!(ecl.entries[0].scripts[0].instrs.iter().any(|instr| instr.opcode == 999));
        assert!(ecl.entries[0].scripts[0].instrs.iter().any(|instr| instr.opcode == 998));
    },
);

source_test!(
    ANM_10, intrinsic_with_novel_abi,
    compile_args: &["--no-builtin-mapfiles"],  // only use intrinsics defined in this test
    mapfile: r#"!anmmap
!ins_signatures
95  SS
99  Sto   # no game has a CountJmp signature like this!!
!ins_intrinsics
95  AssignOp(op="="; type="int")
99  CountJmp()
"#,
    main_body: r#"
    $I0 = 10;
blah:
    +50:
    ins_99($I0, timeof(blah), offsetof(blah));
"#,
    check_decompiled: |decompiled| {
        // should decompile to a `do { } while (--$I0)` loop!
        assert!(decompiled.contains("while (--"));
    },
);

source_test!(
    ANM_10, intrinsic_float_op_like_eosd_ecl,
    compile_args: &["--no-builtin-mapfiles"],  // only use intrinsics defined in this test
    mapfile: r#"!anmmap
!ins_signatures
99  Sff   # EoSD ECL writes output regs as integers
!ins_intrinsics
99  BinOp(op="+"; type="float")
"#,
    main_body: r#"
    ins_99($REG[10], %REG[20], 3.5f);
"#,
    check_decompiled: |decompiled| {
        // The raw ins_ used $REG but the decompiled op should use %REG syntax
        assert!(decompiled.contains("%REG[10] = %REG[20] + 3.5"));
    },
);

source_test!(
    ECL_06, diff_flags_bad_index,
    mapfile: r#"!eclmap
!difficulty_flags
8 b-  //~ ERROR out of range
"#,
    main_body: "",
);

source_test!(
    ECL_06, diff_flags_syntax_errors,
    mapfile: r#"!eclmap
!difficulty_flags
1 @-                         //~ ERROR invalid difficulty
2 X@                         //~ ERROR invalid difficulty
3 a                          //~ ERROR invalid difficulty
4 θ  # a two byte character  //~ ERROR invalid difficulty
"#,
    main_body: "",
);

source_test!(
    ANM_10, multiple_m_arguments,
    compile_args: &[
        "-m", "tests/integration/resources/multiple-mapfiles-1.anmm",
        "-m", "tests/integration/resources/multiple-mapfiles-2.anmm",
    ],
    main_body: r#"
    aaa(2, 4);
    bbb(5, 7);
"#,
    check_compiled: |_, _| {}, // just expecting no warnings/errors
);
