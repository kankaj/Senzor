use rand::SeedableRng;
use rand::rngs::ChaCha8Rng;

// The FNV-1a 64-bit parameters, from the FNV reference.
const FNV_OFFSET_BASIS: u64 = 0xcbf2_9ce4_8422_2325;
const FNV_PRIME: u64 = 0x0000_0100_0000_01b3;

/// 64-bit FNV-1a. Stable across Rust versions, unlike `DefaultHasher`.
fn fnv1a64(data: &[u8]) -> u64 {
    let mut hash = FNV_OFFSET_BASIS;
    for byte in data {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(FNV_PRIME);
    }
    hash
}

/// The RNG that drives one instance's generators, noise and start offset.
pub fn instance_rng(seed: Option<u64>, instance_id: &str) -> ChaCha8Rng {
    match seed {
        Some(seed) => ChaCha8Rng::seed_from_u64(seed ^ fnv1a64(instance_id.as_bytes())),
        None => rand::make_rng(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rand::RngExt;

    /// The first few draws of an instance's RNG.
    fn first_draws(seed: u64, instance_id: &str) -> Vec<u64> {
        let mut rng = instance_rng(Some(seed), instance_id);
        (0..5).map(|_| rng.random_range(0..1_000_000)).collect()
    }

    #[test]
    fn fnv1a64_matches_reference_values() {
        assert_eq!(fnv1a64(b""), 0xcbf2_9ce4_8422_2325);
        assert_eq!(fnv1a64(b"a"), 0xaf63_dc4c_8601_ec8c);
    }

    #[test]
    fn same_seed_and_id_give_same_sequence() {
        assert_eq!(first_draws(42, "office-0"), first_draws(42, "office-0"));
    }

    #[test]
    fn different_ids_give_different_sequences() {
        assert_ne!(first_draws(42, "office-0"), first_draws(42, "office-1"));
    }
}
