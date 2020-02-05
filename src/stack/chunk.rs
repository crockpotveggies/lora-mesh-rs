use log::*;

/// if a packet is too big, split them and return the result
pub fn chunk_data(mut data: Vec<u8>, maxlength: usize) -> Vec<Vec<u8>> {
    let mut ret: Vec<Vec<u8>> = vec![];
    let packetsize = data.len();
    loop {
        if data.len() > maxlength {
            let (first, second) = data.split_at(maxlength);
            ret.push(Vec::from(first));
            data = Vec::from(second);
        }
        else {
            ret.push(data);
            break;
        }
    }
    debug!("Created {} chunks from packet of size {}", ret.len(), packetsize);
    return ret;
}