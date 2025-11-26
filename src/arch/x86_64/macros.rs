#[macro_export]
macro_rules! expand_bool(
    ($value:expr_2021) => {
        concat!($value)
    }
);

/// A macro for generating alternative code sequences.
///
/// This macro is used to generate code that can be patched at runtime to use a more efficient
/// instruction sequence if a certain CPU feature is present.
#[macro_export]
macro_rules! alternative(
    (feature: $feature:literal, then: [$($then:expr_2021),*], default: [$($default:expr_2021),*]) => {
        $crate::alternative2!(feature1: $feature, then1: [$($then),*], feature2: "", then2: [""], default: [$($default),*])
    }
);
#[macro_export]
macro_rules! saturating_sub(
    ($lhs:literal, $rhs:literal) => { concat!(
        "((", $lhs, ")>(", $rhs, "))*((", $lhs, ")-(", $rhs, "))",
    ) }
);
/// A macro for generating alternative code sequences with a fallback.
///
/// This macro is used to generate code that can be patched at runtime to use a more efficient
/// instruction sequence if a certain CPU feature is present. If the feature is not present, a
/// fallback implementation is used.
#[macro_export]
macro_rules! alternative2(
    (feature1: $feature1:literal, then1: [$($then1:expr_2021),*], feature2: $feature2:literal, then2: [$($then2:expr_2021),*], default: [$($default:expr_2021),*]) => {
        concat!("
            .set true, 1
            .set false, 0
            40:
            .if ", $crate::expand_bool!(cfg!(cpu_feature_always = $feature1)), "
            ", $($then1,)* "
            .elseif ", $crate::expand_bool!(cfg!(cpu_feature_always = $feature2)), "
            ", $($then2,)* "
            .else
            ", $($default,)* "
            .endif
            42:
            .if ", $crate::expand_bool!(cfg!(cpu_feature_auto = $feature1)), "
            .skip -", $crate::saturating_sub!("51f - 50f", "42b - 40b"), ", 0x90
            .endif
            .if ", $crate::expand_bool!(cfg!(cpu_feature_auto = $feature2)), "
            .skip -", $crate::saturating_sub!("61f - 60f", "42b - 40b"), ", 0x90
            .endif
            41:
            ",
            // FIXME: The assembler apparently complains "invalid number of bytes" despite it being
            // quite obvious what saturating_sub does.

            // Declare them in reverse order. Last relocation wins!
            $crate::alternative_auto!("6", $feature2, [$($then2),*]),
            $crate::alternative_auto!("5", $feature1, [$($then1),*]),
        )
    };
);
#[macro_export]
macro_rules! alternative_auto(
    ($first_digit:literal, $feature:literal, [$($then:expr_2021),*]) => { concat!(
        ".if ", $crate::expand_bool!(cfg!(cpu_feature_auto = $feature)), "
        .pushsection .altcode.", $feature, ",\"a\"
        ", $first_digit, "0:
        ", $($then,)* "
        ", $first_digit, "1:
        .popsection
        .pushsection .altfeatures.", $feature, ",\"a\"
        70: .ascii \"", $feature, "\"
        71:
        .popsection
        .pushsection .altrelocs.", $feature, ",\"a\"
        .quad 70b
        .quad 71b - 70b
        .quad 40b
        .quad 42b - 40b
        .quad 41b - 40b
        .quad 0
        .quad ", $first_digit, "0b
        .quad ", $first_digit, "1b - ", $first_digit, "0b
        .popsection
        .endif
        ",
    ) }
);
