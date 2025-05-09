#![allow(non_snake_case)]
#![allow(clippy::many_single_char_names)]
#![allow(clippy::zero_prefixed_literal)]
#![allow(dead_code)]

use std::time::Instant;

use std::fmt::Write;
use quorum_proto::scheme_types::Group;
use quorum_schemes::interface::ThresholdScheme;
use quorum_schemes::interface::{ThresholdCipher, ThresholdCipherParams};
use quorum_schemes::keys::key_generator::KeyGenerator;
use quorum_schemes::rand::{RngAlgorithm, RNG};
use quorum_schemes::util::{hex2string, printbinary};

fn main() {
    const K: usize = 30; // threshold
    const N: usize = 50; // total number of secret shares

    // prepare message and label
    let plaintext = "This is a test message!  ";
    let msg: Vec<u8> = String::from(plaintext).as_bytes().to_vec();
    let label = b"Label";

    println!("K: {} N: {}", K, N);
    println!("Message: {}", plaintext);

    // perform threshold encryption using Sg02 scheme
    println!("\n--Sg02 Threshold Cipher--");

    // generate secret shares for Sg02 scheme over Bls12381 curve
    let now = Instant::now();
    let sk = KeyGenerator::generate_keys(
        K,
        N,
        &mut RNG::new(RngAlgorithm::OsRng),
        &ThresholdScheme::Sg02,
        &Group::Bls12381,
        &Option::None,
    )
    .unwrap();
    let elapsed_time = now.elapsed().as_millis();
    println!("[{}ms]\t{} Keys generated", K, elapsed_time);

    // initialize new random number generator
    let mut params = ThresholdCipherParams::new();

    // a public key is stored inside each secret share, so those can be used for encryption
    let now = Instant::now();
    let ciphertext =
        ThresholdCipher::encrypt(&msg, label, &sk[0].get_public_key(), &mut params).unwrap();
    let encrypt_time = now.elapsed().as_millis();
    println!("[{}ms]\tMessage encrypted", encrypt_time);

    let mut s = String::with_capacity(25);
    write!(&mut s, "[{}ms]\tCiphertext: ", encrypt_time).expect("error");
    printbinary(&ciphertext.get_ctxt(), Some(s.as_str()));

    // check whether ciphertext is valid
    let now = Instant::now();
    let valid = ThresholdCipher::verify_ciphertext(&ciphertext, &sk[0].get_public_key()).unwrap();
    let elapsed_time = now.elapsed().as_millis();

    // create decryption shares and verify them
    let mut shares = Vec::new();
    let mut share_gen_time = 0;
    let mut share_verify_time = 0;

    println!("[*] Generating and validating shares...");

    for i in 0..K {
        let now = Instant::now();
        shares.push(
            ThresholdCipher::partial_decrypt(&ciphertext, &sk[i as usize], &mut params).unwrap(),
        );
        share_gen_time += now.elapsed().as_millis();

        let now = Instant::now();
        let valid = ThresholdCipher::verify_share(
            &shares[i as usize],
            &ciphertext,
            &sk[0].get_public_key(),
        )
        .unwrap();
        share_verify_time += now.elapsed().as_millis();
    }
    println!("[{}ms]\t{} Shares generated", share_gen_time, K);
    println!("[{}ms]\t{} Shares validated", share_verify_time, K);

    println!("[*] Decrypting...");
    // assemble decryption shares to restore original message
    let now = Instant::now();
    let msg = ThresholdCipher::assemble(&shares, &ciphertext).unwrap();
    let elapsed_time = now.elapsed().as_millis();

    println!(
        "[{}ms]\tMessage decrypted: {}",
        elapsed_time,
        hex2string(&msg)
    );
}
