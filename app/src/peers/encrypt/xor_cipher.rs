use crate::tunnel::packet_def::ZCPacket;

use super::{Encryptor, Error};

// XOR 加密不需要额外的尾部数据，因为它是对称的
pub const XOR_ENCRYPTION_RESERVED: usize = 0;

#[derive(Clone)]
pub struct XorCipher {
    pub(crate) key: Vec<u8>,
}

impl XorCipher {
    pub fn new(key: &[u8]) -> Self {
        if key.is_empty() {
            panic!("XOR key cannot be empty");
        }
        Self { key: key.to_vec() }
    }

    fn xor_data(&self, data: &mut [u8]) {
        for (i, byte) in data.iter_mut().enumerate() {
            *byte ^= self.key[i % self.key.len()];
        }
    }
}

impl Encryptor for XorCipher {
    fn decrypt(&self, zc_packet: &mut ZCPacket) -> Result<(), Error> {
        let pm_header = zc_packet.peer_manager_header().unwrap();
        if !pm_header.is_encrypted() {
            return Ok(());
        }

        // XOR 解密（XOR是对称的，加密和解密操作相同）
        self.xor_data(zc_packet.mut_payload());

        let pm_header = zc_packet.mut_peer_manager_header().unwrap();
        pm_header.set_encrypted(false);

        Ok(())
    }

    fn encrypt(&self, zc_packet: &mut ZCPacket) -> Result<(), Error> {
        let pm_header = zc_packet.peer_manager_header().unwrap();
        if pm_header.is_encrypted() {
            tracing::warn!(?zc_packet, "packet is already encrypted");
            return Ok(());
        }

        // XOR 加密
        self.xor_data(zc_packet.mut_payload());

        let pm_header = zc_packet.mut_peer_manager_header().unwrap();
        pm_header.set_encrypted(true);

        Ok(())
    }
}
