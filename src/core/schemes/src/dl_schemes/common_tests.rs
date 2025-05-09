use crate::{
    dl_schemes::common::{eval_pol, horner, shamir_share},
    rand::{RngAlgorithm, RNG},
};

use quorum_proto::scheme_types::Group;

use crate::{groups::ec::bls12381::Bls12381, integers::sizedint::SizedBigInt};

const GROUP: Group = Group::Bls12381;

#[test]
fn test_shamir_share() {
    let x = SizedBigInt::new_int(&GROUP, 5);

    let mut rng = RNG::new(RngAlgorithm::OsRng);
    let (c, d) = shamir_share(&x, 2, 3, &mut rng);
    assert!(c.len() == 3);
    assert!(d.len() == 3);
}

#[test]
fn test_eval_pol() {
    let mut x = SizedBigInt::new_int(&GROUP, 2);
    let mut a = Vec::new();
    a.push(SizedBigInt::new_int(&GROUP, 1));
    a.push(SizedBigInt::new_int(&GROUP, 2));
    a.push(SizedBigInt::new_int(&GROUP, 3));

    let res = eval_pol(&mut x, &a).rmod(&SizedBigInt::new_int(&GROUP, 7));
    let c = SizedBigInt::new_int(&GROUP, 4);

    assert!(res.equals(&c));
}

#[test]
fn test_horner() {
    let mut x = SizedBigInt::new_int(&GROUP, 2);
    let mut a = Vec::new();
    a.push(SizedBigInt::new_int(&GROUP, 1));
    a.push(SizedBigInt::new_int(&GROUP, 2));
    a.push(SizedBigInt::new_int(&GROUP, 3));

    let res = horner(&mut x, &a).rmod(&SizedBigInt::new_int(&GROUP, 7));
    let c = SizedBigInt::new_int(&GROUP, 4);

    assert!(res.equals(&c));
}
