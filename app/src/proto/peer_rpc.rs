use hmac::{Hmac, Mac};
use sha2::Sha256;

use crate::common::PeerId;

include!(concat!(env!("OUT_DIR"), "/peer_rpc.rs"));

impl PeerGroupInfo {
    pub fn generate_with_proof(group_name: String, group_secret: String, peer_id: PeerId) -> Self {
        let mut mac = Hmac::<Sha256>::new_from_slice(group_secret.as_bytes())
            .expect("HMAC can take key of any size");

        let mut data_to_sign = group_name.as_bytes().to_vec();
        data_to_sign.push(0x00); // Add a delimiter byte
        data_to_sign.extend_from_slice(&peer_id.to_be_bytes());

        mac.update(&data_to_sign);

        let proof = mac.finalize().into_bytes().to_vec();

        PeerGroupInfo {
            group_name,
            group_proof: proof,
        }
    }

    pub fn verify(&self, group_secret: &str, peer_id: PeerId) -> bool {
        let mut verifier = Hmac::<Sha256>::new_from_slice(group_secret.as_bytes())
            .expect("HMAC can take key of any size");

        let mut data_to_sign = self.group_name.as_bytes().to_vec();
        data_to_sign.push(0x00); // Add a delimiter byte
        data_to_sign.extend_from_slice(&peer_id.to_be_bytes());

        verifier.update(&data_to_sign);

        verifier.verify_slice(&self.group_proof).is_ok()
    }
}
