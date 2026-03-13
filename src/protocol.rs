use futures::{AsyncReadExt, AsyncWriteExt};
use libp2p::PeerId;

/// Wire message types.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum MessageType {
    Unexpected = 1,
    Expected = 2,
    RmaPut = 3,
    RmaGetReq = 4,
    RmaGetResp = 5,
}

impl MessageType {
    pub fn from_u8(v: u8) -> Option<Self> {
        match v {
            1 => Some(Self::Unexpected),
            2 => Some(Self::Expected),
            3 => Some(Self::RmaPut),
            4 => Some(Self::RmaGetReq),
            5 => Some(Self::RmaGetResp),
            _ => None,
        }
    }
}

/// Fixed-size header (on the wire).
pub struct WireHeader {
    pub msg_type: MessageType,
    pub tag: u32,
    pub payload_size: u32,
    pub source_peer_id: PeerId,
    pub handle_id: u64,
    pub offset: u64,
    pub rma_length: u64,
    pub local_handle_id: u64,
    pub local_offset: u64,
}

/// Write a complete message (header + payload) to a stream.
pub async fn write_message<S: AsyncWriteExt + Unpin>(
    stream: &mut S,
    header: &WireHeader,
    payload: &[u8],
) -> std::io::Result<()> {
    let mut buf = Vec::with_capacity(64 + payload.len());

    // msg_type
    buf.push(header.msg_type as u8);
    // tag
    buf.extend_from_slice(&header.tag.to_be_bytes());
    // payload_size
    buf.extend_from_slice(&header.payload_size.to_be_bytes());
    // source peer id (length-prefixed)
    let peer_bytes = header.source_peer_id.to_bytes();
    buf.extend_from_slice(&(peer_bytes.len() as u16).to_be_bytes());
    buf.extend_from_slice(&peer_bytes);
    // RMA fields
    buf.extend_from_slice(&header.handle_id.to_be_bytes());
    buf.extend_from_slice(&header.offset.to_be_bytes());
    buf.extend_from_slice(&header.rma_length.to_be_bytes());
    buf.extend_from_slice(&header.local_handle_id.to_be_bytes());
    buf.extend_from_slice(&header.local_offset.to_be_bytes());
    // payload
    buf.extend_from_slice(payload);

    stream.write_all(&buf).await?;
    stream.flush().await?;
    Ok(())
}

/// Read a complete message (header + payload) from a stream.
pub async fn read_message<S: AsyncReadExt + Unpin>(
    stream: &mut S,
) -> std::io::Result<(WireHeader, Vec<u8>)> {
    // Read msg_type (1 byte)
    let mut b1 = [0u8; 1];
    stream.read_exact(&mut b1).await?;
    let msg_type = MessageType::from_u8(b1[0])
        .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::InvalidData, "bad msg type"))?;

    // tag (4 bytes)
    let mut b4 = [0u8; 4];
    stream.read_exact(&mut b4).await?;
    let tag = u32::from_be_bytes(b4);

    // payload_size (4 bytes)
    stream.read_exact(&mut b4).await?;
    let payload_size = u32::from_be_bytes(b4);

    // source peer id (2 byte length + bytes)
    let mut b2 = [0u8; 2];
    stream.read_exact(&mut b2).await?;
    let peer_id_len = u16::from_be_bytes(b2) as usize;
    let mut peer_id_buf = vec![0u8; peer_id_len];
    stream.read_exact(&mut peer_id_buf).await?;
    let source_peer_id = PeerId::from_bytes(&peer_id_buf)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;

    // RMA fields (5 x 8 bytes)
    let mut b8 = [0u8; 8];
    stream.read_exact(&mut b8).await?;
    let handle_id = u64::from_be_bytes(b8);
    stream.read_exact(&mut b8).await?;
    let offset = u64::from_be_bytes(b8);
    stream.read_exact(&mut b8).await?;
    let rma_length = u64::from_be_bytes(b8);
    stream.read_exact(&mut b8).await?;
    let local_handle_id = u64::from_be_bytes(b8);
    stream.read_exact(&mut b8).await?;
    let local_offset = u64::from_be_bytes(b8);

    // payload
    let mut payload = vec![0u8; payload_size as usize];
    if payload_size > 0 {
        stream.read_exact(&mut payload).await?;
    }

    Ok((
        WireHeader {
            msg_type,
            tag,
            payload_size,
            source_peer_id,
            handle_id,
            offset,
            rma_length,
            local_handle_id,
            local_offset,
        },
        payload,
    ))
}
