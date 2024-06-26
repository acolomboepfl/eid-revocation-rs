use super::{Accumulator, Coefficient, Element, Error, PolynomialG1, PublicKey, SecretKey};
use bls12_381_plus::{multi_miller_loop, G1Affine, G1Projective, G2Prepared, G2Projective, Scalar};
use core::{convert::TryFrom, fmt};
use group::{Curve, Group, GroupEncoding};
use serde::{Deserialize, Serialize};

// Groups the new accumulator value and the deleted element after
#[derive(Copy, Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct Deletion(pub Accumulator, pub Element);

/// A membership witness that can be used for membership proof generation
/// as described in section 4 in
/// <https://eprint.iacr.org/2020/777>
#[derive(Copy, Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct MembershipWitness(pub G1Projective);

impl fmt::Display for MembershipWitness {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "MembershipWitness {{ {} }}", self.0)
    }
}

impl From<MembershipWitness> for G1Projective {
    fn from(m: MembershipWitness) -> Self {
        m.0
    }
}

impl From<G1Projective> for MembershipWitness {
    fn from(g: G1Projective) -> Self {
        Self(g)
    }
}

impl TryFrom<&[u8; 48]> for MembershipWitness {
    type Error = Error;

    fn try_from(value: &[u8; 48]) -> Result<Self, Self::Error> {
        let pt = G1Affine::from_compressed(value).map(G1Projective::from);
        if pt.is_some().unwrap_u8() == 1 {
            Ok(Self(pt.unwrap()))
        } else {
            Err(Error {
                message: String::from("incorrect byte sequence"),
                code: 1,
            })
        }
    }
}

impl MembershipWitness {
    const BYTES: usize = 80;

    /// Compute the witness using a prehashed element
    pub fn new(value: &Element, accumulator: Accumulator, secret_key: &SecretKey) -> Self {
        Self(accumulator.remove(secret_key, *value).0)
    }


    /// Membership witness update as defined in section 3 of <https://eprint.iacr.org/2022/1362>.
    /// Return a new witness
    pub fn update(&self, y: Element, del: &[Deletion]) -> Self {
        let mut clone = *self;
        clone.update_assign(y, del);
        clone
    }

    /// Perform in place witness update as defined in section 3 of <https://eprint.iacr.org/2022/1362>
    pub fn update_assign(&mut self, y: Element, del: &[Deletion]) {
        // C' = 1/(y' - y) (C - V')
        for d in del {
            let mut inv = d.1.0 - y.0;
            // If this fails, then this value was removed
            let t = inv.invert();
            if bool::from(t.is_none()) {
                return;
            }
            inv = t.unwrap();
            self.0 -= d.0 .0;
            self.0 *= inv;
        }
    }

    /// Perform batch update using the associated element `y`, the list of coefficients `omega`, 
    /// and list of deleted elements `deletions`.
    /// 
    /// Returns a new updated instance of `MembershipWitness`.
    pub fn batch_update(
        &self,
        y: Element,
        deletions: &[Element],
        omega: &[Coefficient],
    ) -> Result<MembershipWitness, Error>
    {
        return self.clone().batch_update_assign(y, deletions, omega);
    }

    /// Perform batch update of the witness in-place
    /// using the associated element `y`, the list of coefficients `omega`, 
    /// and list of deleted elements `deletions`.
    pub fn batch_update_assign(
        &mut self,
        y: Element,
        deletions: &[Element],
        omega: &[Coefficient],
    ) -> Result<MembershipWitness, Error>
    {
        // dD(x) = ∏ 1..m (yD_i - x)
        let mut d_d = dd_eval(deletions.as_ref(), y.0);

        let t = d_d.invert();
        // If this fails, then this value was removed
        if bool::from(t.is_none()) {
            return Err(Error::from_msg(1, "no inverse exists"));
        }
        d_d = t.unwrap();

        let poly = PolynomialG1(
            omega
                .as_ref()
                .iter()
                .map(|c| c.0)
                .collect::<Vec<G1Projective>>(),
        );

        // Compute〈Υy,Ω〉using Multi Scalar Multiplication
        if let Some(v) = poly.msm(&y.0) {
            // C' = 1 / dD * (C -〈Υy,Ω))
            self.0 -= v;
            self.0 *= d_d;
            Ok(*self)
        } else {
            Err(Error::from_msg(2, "polynomial could not be evaluated"))        
        }
    }

    /// Substitutes the underlying G1 point with the `new_wit` given as input.
    pub fn apply_update(&mut self, new_wit: G1Projective) {
        self.0 = new_wit;
    }

    /// Verify this is a valid witness for element `y`, public key `pubkey`, and accumulator value `accumulator`.
    pub fn verify(&self, y: Element, pubkey: PublicKey, accumulator: Accumulator) -> bool {
        let mut p = G2Projective::GENERATOR;
        p *= y.0;
        p += pubkey.0;
        let g2 = G2Projective::GENERATOR;
        
        // Notation as per section 2 in <https://eprint.iacr.org/2020/777>
        // e(C, yP~ + Q~) == e(V, P~) <=>  e(C, yP~ + Q~) - e(V, P~) == 0_{G_t}
        bool::from(
            multi_miller_loop(&[
                // e(C, yP~ + Q~)
                (&self.0.to_affine(), &G2Prepared::from(p.to_affine())),
                // -e(V, P~)
                (
                    &accumulator.0.to_affine(),
                    &G2Prepared::from(-g2.to_affine()),
                ),
            ])
            .final_exponentiation()
            .is_identity(),
        )
    }

    /// Return the byte sequence for this witness.
    pub fn to_bytes(&self) -> [u8; Self::BYTES] {
        let mut res = [0u8; Self::BYTES];
        res.copy_from_slice(self.0.to_bytes().as_ref());
        res
    }

    /// Old unoptimized version, just for testing
    fn _batch_update_assign(
        &mut self,
        y: Element,
        deletions: &[Element],
        coefficients: &[Coefficient],
    ) -> Result<MembershipWitness, Error>{
        // dD(x) = ∏ 1..m (yD_i - x)
        let mut d_d = dd_eval(deletions.as_ref(), y.0);

        let t = d_d.invert();
        // If this fails, then this value was removed
        if bool::from(t.is_none()) {
            return Err(Error::from_msg(1, "no inverse exists"));
        }
        d_d = t.unwrap();

        let poly = PolynomialG1(
            coefficients
                .as_ref()
                .iter()
                .map(|c| c.0)
                .collect::<Vec<G1Projective>>(),
        );

        // Compute〈Υy,Ω〉using direct evaluation
        if let Some(v) = poly.evaluate(&y.0) {
            // C' = 1 / dD * (C -〈Υy,Ω))
            self.0 -= v;
            self.0 *= d_d;
            Ok(*self)
        } else {
            Err(Error::from_msg(2, "polynomial could not be evaluated")) 
        }
    }

}

/// Evaluates poly dD(y) = ∏ 1..m (yD_i - y)
fn dd_eval(values: &[Element], y: Scalar) -> Scalar {
    if values.len() == 1 {
        values[0].0 - y
    } else {
        values
            .iter()
            .map(|v| v.0 - y)
            .fold(Scalar::ONE, |a, y| a * y)
    }
}

#[cfg(test)]
mod tests {
    use rand::rngs::OsRng;
    use serde::de::IntoDeserializer;

    use super::*;
    use crate::key;
    use std::time::Instant;
    use std::time::SystemTime;

    fn init(upd_size: usize) -> (key::SecretKey, key::PublicKey, Accumulator, Vec<Element>) {
        let key = SecretKey::new(Some(b"1234567890"));
        let pubkey = PublicKey::from(&key);
        let mut elements = vec![Element::one(); upd_size];

        (0..upd_size).for_each(|i| elements[i] = Element::hash(i.to_string().as_bytes()));

        let acc = Accumulator::random(rand_core::OsRng {});
        (key, pubkey, acc, elements)
    }

    fn wit_sequential_update(upd_size: usize) {
        let (key, pubkey, mut acc, elements) = init(upd_size + 1);

        // Non revoked (y,C) pair
        let elem = elements[0];
        let mut wit = MembershipWitness::new(&elem, acc, &key);

        // Revoked (y,C) pair
        let elem_d = elements[1];
        let mut wit_d = MembershipWitness::new(&elem_d, acc, &key);

        // Revoke everyone except for elem
        let dels = &elements[1..upd_size];
        let mut deletions: Vec<Deletion> = Vec::new();
        dels.iter().for_each(|&d| {
            acc.remove_assign(&key, d);
            deletions.push(Deletion { 0: acc, 1: d });
        });

        // Update non-revoked element
        let t = Instant::now();
        wit.update_assign(elem, &deletions.as_slice());
        let t = t.elapsed();

        // Try update revoked elem
        wit_d.update_assign(elem_d, &deletions.as_slice());

        assert!(wit.verify(elem, pubkey, acc));
        assert!(!wit.verify(elem_d, pubkey, acc));

        println!(
            "Sequential update of {} deletions: {:?}",
            deletions.len(),
            t
        );
    }

    fn wit_batch_update(upd_size: usize) {
        let (key, pubkey, mut acc, elements) = init(upd_size);

        // Non revoked (y, wit) pair
        let y = elements[0];
        let mut wit = MembershipWitness::new(&y, acc, &key);

        // Revoked (y_d, wit_d) pair
        let y_d = elements[1];
        let mut wit_d = MembershipWitness::new(&y_d, acc, &key);

        // Revoke y_1, ..., y_(upd_size-1) and compute coefficients for batch update
        let deletions = &elements[1..];
        let coefficients = acc.update_assign(&key, deletions);

        // Update non-revoked element with both versions
        let mut wit2 = wit.clone();
        let t1 = Instant::now();
        wit.batch_update_assign(y, deletions, &coefficients).expect("Error when evaluating poly");
        let t1 = t1.elapsed();
        let t2 = Instant::now();
        wit2._batch_update_assign(y, deletions, &coefficients).expect("Error when evaluating poly");
        let t2 = t2.elapsed();

        // Try updating revoked element
        wit_d.batch_update_assign(y_d, deletions, &coefficients);

        // Check (non)revocation of updated witness
        assert!(!wit_d.verify(y_d, pubkey, acc));
        assert!(wit.verify(y, pubkey, acc));

        println!("Batch update of {} deletions without MSM: {:?}", deletions.len(), t2);
        println!("Batch update of {} deletions with MSM: {:?}", deletions.len(), t1);
    }

    // Test sequential and batch updates
    #[test]
    fn wit_test_update() {
        let upd_size = 1_001;
        wit_sequential_update(upd_size);
        wit_batch_update(upd_size);
    }

    // Test serialization
    #[test]
    fn wit_test_serialize() {

        // Init parameters
        let sk = SecretKey::new(Some(b"test"));
        let acc = Accumulator::random(rand_core::OsRng {});
        let wit = MembershipWitness::new(&Element::hash(b"test"), acc, &sk);

        // Try serialize and deserialize
        let bytes = bincode::serialize(&wit).expect("Serialization error!");
        let wit = bincode::deserialize::<MembershipWitness>(&bytes).expect("Deserialization error");
        
        // Check witness verifies
        assert!(wit.verify(Element::hash(b"test"), PublicKey::from(&sk), acc))
    }
}
