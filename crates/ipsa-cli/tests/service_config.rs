use ipsa_cli::services::{
    cosmos::config::{fee_at_price, DEFAULT_GAS_PRICE},
    stellar::config::split_list,
};

#[test]
fn the_default_price_covers_what_simd_demanded() {
    let price: f64 = DEFAULT_GAS_PRICE.parse().unwrap();

    assert_eq!(fee_at_price(2_000_000, price), 50_000);
    assert_eq!(fee_at_price(400_000, price), 10_000);
}

#[test]
fn a_fee_is_rounded_up_so_it_is_never_short() {
    assert_eq!(fee_at_price(1, 0.025), 1);
    assert_eq!(fee_at_price(41, 0.025), 2);
}

#[test]
fn a_zero_or_nonsense_price_asks_for_no_fee() {
    assert_eq!(fee_at_price(2_000_000, 0.0), 0);
    assert_eq!(fee_at_price(2_000_000, -1.0), 0);
    assert_eq!(fee_at_price(2_000_000, f64::NAN), 0);
}

#[test]
fn an_unset_list_is_empty_not_a_blank_entry() {
    assert!(split_list("").is_empty());
    assert!(split_list("  ,  ").is_empty());
}

#[test]
fn entries_are_split_and_trimmed() {
    assert_eq!(
        split_list(" a , b "),
        vec!["a".to_string(), "b".to_string()]
    );
}
