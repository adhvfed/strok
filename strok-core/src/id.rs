use rand::Rng;
use std::collections::HashMap;

use crate::node::NodeId;

const BASE_LEN: usize = 8;
const CHARSET: &[u8] = b"abcdefghijklmnopqrstuvwxyz0123456789";
const MAX_RETRIES: usize = 10;

pub fn generate_id(existing: &HashMap<String, NodeId>) -> String {
    let mut rng = rand::thread_rng();
    let mut len = BASE_LEN;

    for attempt in 0.. {
        if attempt > 0 && attempt % MAX_RETRIES == 0 {
            len += 1;
        }
        let id: String = (0..len)
            .map(|_| CHARSET[rng.gen_range(0..CHARSET.len())] as char)
            .collect();
        if !existing.contains_key(&id) && id != "root" {
            return id;
        }
    }
    unreachable!()
}

pub fn validate_id(id: &str, existing: &HashMap<String, NodeId>) -> crate::error::Result<()> {
    if id == "root" {
        return Err(crate::error::StrokError::ReservedId);
    }
    if existing.contains_key(id) {
        return Err(crate::error::StrokError::IdConflict(id.to_string()));
    }
    Ok(())
}
