use accumulator::{
    accumulator::Element, proof::{self, Proof, ProofParamsPublic, PROOF_LABEL}, witness::{MembershipWitness, UpMsg}, Accumulator, Coefficient, Error, ProofParamsPrivate
};
use crate::{issuer::RevocationHandle, UpdatePolynomials};
use crate::Updatable;

use blsful::inner_types::Scalar;

#[derive(Debug)]
pub struct Holder {
    pseudo: String,
    y: Element,
    w: MembershipWitness,
    pp: ProofParamsPublic,
}

impl Holder {

    /// Returns a new `Holder` instance, associated with pseudonym `pseudo`,
    /// the revocation handle `rh`, and the public parameters `pp` to be used for creating non-revocation proofs.
    pub fn new<T: Into<String>>(pseudo: T, rh: RevocationHandle, pp: ProofParamsPublic) -> Self {
        let pseudo: String = pseudo.into();
        Self { pseudo, y: rh.get_elem(), w: rh.get_witness(), pp}
    }

    /// Sequentially updates the witness using the vector of deletions `rh`.
    pub fn update(&mut self, del: &[UpMsg]){
        self.w.update_assign(self.y, del);
    }

    /// Batch update the holder's witness with the update polynomials received as input.
    pub fn batch_update(& mut self, update_poly: &UpdatePolynomials) -> Result<MembershipWitness, Error>{
        self.w.batch_update_assign(self.y, &update_poly.deletions, &update_poly.omegas)
    }

    /// Aggregate multiples batch updates to the holder's witness 
    /// using the array update polynomials received as input.
    pub fn batch_updates(& mut self, update_poly: &[UpdatePolynomials]) -> Result<MembershipWitness, Error>{  
        let deletions: Vec<&[Element]> = update_poly.iter().map(|up| up.deletions.as_slice()).collect();
        let coefficients: Vec<&[Coefficient]> = update_poly.iter().map(|up| up.omegas.as_slice()).collect();
        self.w.batch_update_aggr_assign(self.y, &deletions, coefficients)
    }
    
    /// Replace the holder's witness with the input witness `new_mw`.
    pub fn replace_witness(&mut self, new_mw: MembershipWitness) {
        self.w = new_mw;
    }

    /// Test membership of the holder's witness against the accumulator contained 
    /// in the proof parameters `pp` received as input or in the cached parameters. 
    pub fn test_membership(&self, pub_params: Option<ProofParamsPublic>)->bool{
        let pp = if pub_params.is_some() { pub_params.unwrap() } else { self.pp };
        self.w.verify(self.y, pp.get_public_key(), pp.get_accumulator())
    }

    /// Creates a new membership proof using either the optional input parameters or the cached parameters.
    pub fn proof_membership(&self, pub_params: Option<ProofParamsPublic>) -> Proof {
        let pp = if pub_params.is_some() { pub_params.unwrap() } else { self.pp };
        let mut transcript = merlin::Transcript::new(PROOF_LABEL);
        pp.add_to_transcript(&mut transcript);

        let priv_params = ProofParamsPrivate::new(self.y, &self.w);
        let pc = proof::ProofCommitting::new(&pp, &priv_params);
        pc.get_bytes_for_challenge(&mut transcript);

        let challenge_hash = Element::from_transcript(PROOF_LABEL, &mut transcript);
        return pc.gen_proof(challenge_hash);
    }

    /// Returns the id of the the holder's accumulator.
    pub fn get_accumulator_id(&self) -> Scalar{
        self.pp.get_accumulator().get_id()
    }

    /// Returns the pseudonym associated to the holder.
    pub fn get_pseudo(&self) -> String{
        return self.pseudo.clone();
    }
}

impl Updatable for Holder{

    /// Update the holder's public parameters with the new parameters `new_pp`.
    fn update_public_params(&mut self, new_pp: ProofParamsPublic) {
        self.pp = new_pp;
    }

    /// Update the holder's accumulator with the new accumulator `new_acc`.
    fn update_accumulator(&mut self, new_acc: Accumulator) {
        self.pp.update_accumulator(new_acc);
    }
}


#[cfg(test)]
mod tests {
    use crate::holder::Holder;
    use crate::issuer::Issuer;
    use std::time::Instant;
    const ADD_SIZE: usize = 1001;

    #[test]
    fn holder_single_update() {
        // Setup issuer
        let mut issuer = Issuer::new(None);
        let pp = issuer.get_proof_params();
   
        let mut holders = Vec::new();  
        
        // Compute witnesses for ADD_SIZE elements
        (0..ADD_SIZE).for_each(|i| {
            let rh = issuer.add(i.to_string()).expect("Cannot add witness");
            holders.push(Holder::new(i.to_string(), rh, pp));
        });

        // Delete one of the elements, compute update and get updated params
        let polys = issuer.revoke_instant(&1.to_string()).expect("Non existing element");
        let pp = issuer.get_proof_params();

        // Check non-revoked holder is invalid before updating and is valid after updating
        let valid_hol = &mut holders[0];
        assert!(!valid_hol.test_membership(Some(pp)));
        let t = Instant::now();
        assert!(valid_hol.batch_update(&polys).is_ok());
        let t = t.elapsed();
        assert!(valid_hol.test_membership(Some(pp)));
        println!("Time to update witness using polys after single update: {:?}",
            t
        );

        // Check revoked holder is always invalid
        let revoked_hol = &mut holders[1];
        assert!(!revoked_hol.test_membership(Some(pp)));
        assert!(revoked_hol.batch_update(&polys).is_err());
        assert!(!revoked_hol.test_membership(Some(pp)));
    }

    
    #[test]
    fn holder_batch_update() {
        // Setup issuer
        let mut issuer = Issuer::new(None);
        let pp = issuer.get_proof_params();

        // Add ADD_SIZE holders
        let mut holders = Vec::with_capacity(ADD_SIZE);  
        (0..ADD_SIZE).for_each(|i| {
            let rh = issuer.add(i.to_string()).expect("Cannot add witness");
            holders.push(Holder::new(i.to_string(), rh, pp));
        });

        // Delete one of the elements, compute update and get updated params
        let revoked: Vec<String> = (1..ADD_SIZE).map(|i|i.to_string()).collect();
        let polys = issuer.revoke_elements_instant(&revoked.as_slice()).expect("Non existing element");
        let pp = issuer.get_proof_params();

        // Check non-revoked holder is invalid before updating and is valid after updating
        let valid_hol = & mut holders[0];
        assert!(!valid_hol.test_membership(Some(pp)));
        let t = Instant::now();
        let res = valid_hol.batch_update(&polys);
        let t = t.elapsed();
        assert!(res.is_ok() && valid_hol.test_membership(Some(pp)));
        println!("Time to update witness after {} revocations in single batch: {:?}",
            polys.deletions.len(),
            t
        );

        // Check revoked holder is always invalid
        let revoked_hol = &mut holders[1];
        assert!(!revoked_hol.test_membership(Some(pp)));
        assert!(revoked_hol.batch_update(&polys).is_err());
        assert!(!revoked_hol.test_membership(Some(pp)));
    }

    
    
    #[test]
    fn holder_multiple_batch_update() {
        // Setup issuer
        let mut issuer = Issuer::new(None);
        let pp = issuer.get_proof_params();

        // Add ADD_SIZE holders
        let mut holders = Vec::with_capacity(ADD_SIZE);  
        (0..ADD_SIZE).for_each(|i| {
            let rh = issuer.add(i.to_string()).expect("Cannot add witness");
            holders.push(Holder::new(i.to_string(), rh, pp));
        });

        let mut polys = Vec::new();

        // Delete one of the elements and compute update
        polys.push(issuer.revoke_instant(&1.to_string()).expect("Non existing element"));

        // Delete one of the elements without updating
        const CHUNK_SIZE: usize = 100;
        let revoked_pseudos: Vec<String> =  (1..ADD_SIZE).map(|i| i.to_string()).collect();
        let revoked_pseudos: Vec<&[String]> = revoked_pseudos.chunks(CHUNK_SIZE).collect();
        for pseudos in revoked_pseudos{
            polys.push(issuer.revoke_elements_instant(pseudos).expect("Non existing element"));
        }
        
        // Check non-revoked holder is invalid before updating and is valid after updating
        let pp = issuer.get_proof_params();
        let valid_hol = &mut holders[0];
        assert!(!valid_hol.test_membership(Some(pp)));
        
        let t = Instant::now();
        let res = valid_hol.batch_updates(polys.as_slice());
        let t = t.elapsed();
        assert!(res.is_ok() && valid_hol.test_membership(Some(pp)));
        println!("Time to update witness after {} revocations in {} batches of {} elements: {:?}",
            ADD_SIZE-1,
            (((ADD_SIZE-1) as f64)/ (CHUNK_SIZE as f64)).ceil(),
            CHUNK_SIZE,
            t
        );

        // Check revoked holder is always invalid
        let revoked_hol = &mut holders[1];
        assert!(!revoked_hol.test_membership(Some(pp)));
        assert!(revoked_hol.batch_updates(polys.as_slice()).is_err());
        assert!(!revoked_hol.test_membership(Some(pp)));
    }
}
