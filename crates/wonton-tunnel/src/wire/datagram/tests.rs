use super::*;

const MTU: usize = 1162;

fn pair(packet: Packet<'_>) -> [Frame<'_>; 2] {
    match packet {
        Packet::Pair(frames) => frames,
        Packet::Whole(_) => panic!("expected a split packet"),
    }
}

#[test]
fn complete_limit_and_one_byte_over_for_both_flow_widths() {
    for value in [63, 64] {
        let flow = FlowId::new(value).unwrap();
        let header_len = flow.encoded_len() + 1;
        let payload = vec![42; MTU - header_len + 1];
        let Packet::Whole(frame) = split(flow, 7, &payload[..payload.len() - 1], MTU).unwrap()
        else {
            panic!("exact limit must be unsplit");
        };
        assert_eq!(frame.header_len() + frame.payload.len(), MTU);
        assert_eq!(frame.payload.as_ptr(), payload.as_ptr());
        let mut output = [0; MTU];
        assert_eq!(frame.encode(&mut output, MTU), Ok(MTU));
        let decoded = Frame::decode(&output).unwrap();
        assert_eq!(decoded, frame);
        assert_eq!(decoded.payload.as_ptr(), output[header_len..].as_ptr());
        assert_eq!(
            frame.encode_header(&mut output, MTU - 1),
            Err(CodecError::PacketTooLarge)
        );
        let frames = pair(split(flow, 8, &payload, MTU).unwrap());
        assert_eq!(payload.len(), MTU - header_len + 1);
        assert_eq!(
            frames[0].payload.len() + frames[1].payload.len(),
            payload.len()
        );
        assert_eq!(frames[0].payload.as_ptr(), payload.as_ptr());
        assert_eq!(
            frames[1].payload.as_ptr(),
            payload[frames[0].payload.len()..].as_ptr()
        );
    }
}

#[test]
fn exact_two_fragment_limit_and_one_byte_over() {
    for value in [63, 64] {
        let flow = FlowId::new(value).unwrap();
        let header_len = flow.encoded_len() + 1 + 8;
        let budget = MTU - header_len;
        let payload = vec![9; budget * 2 + 1];
        assert_eq!(payload.len(), 2 * (MTU - header_len) + 1);
        let frames = pair(split(flow, u64::MAX, &payload[..budget * 2], MTU).unwrap());
        let mut output = [0; MTU];
        for frame in frames {
            assert_eq!(frame.payload.len(), budget);
            assert_eq!(frame.encode(&mut output, MTU), Ok(MTU));
            assert_eq!(Frame::decode(&output).unwrap(), frame);
        }
        assert_eq!(
            split(flow, 1, &payload, MTU),
            Err(CodecError::PacketTooLarge)
        );
    }
}

#[test]
fn reordered_pair_joins_and_missing_part_never_emits_packet() {
    let flow = FlowId::new(64).unwrap();
    let payload: Vec<_> = (0..1200).map(|index| (index % 251) as u8).collect();
    let frames = pair(split(flow, 13, &payload, MTU).unwrap());
    let mut first = [0; MTU];
    let mut second = [0; MTU];
    let first_len = frames[0].encode(&mut first, MTU).unwrap();
    let second_len = frames[1].encode(&mut second, MTU).unwrap();
    let a = Frame::decode(&first[..first_len]).unwrap();
    let b = Frame::decode(&second[..second_len]).unwrap();
    let mut output = vec![0; payload.len()];
    assert_eq!(
        join(Some(a), Some(b), &mut output).unwrap().unwrap(),
        payload
    );
    assert_eq!(
        join(Some(b), Some(a), &mut output).unwrap().unwrap(),
        payload
    );
    assert_eq!(join(Some(a), None, &mut output), Ok(None));
    assert_eq!(join(None, Some(b), &mut output), Ok(None));
    assert_eq!(join(None, None, &mut output), Ok(None));
    assert_eq!(
        join(Some(a), Some(b), &mut output[..payload.len() - 1]),
        Err(CodecError::BufferTooSmall)
    );
}

#[test]
fn duplicates_and_overlapping_packets_cannot_mix() {
    let flow = FlowId::new(5).unwrap();
    let payload = [7; 1200];
    let [a, b] = pair(split(flow, 20, &payload, MTU).unwrap());
    let [next_a, next_b] = pair(split(flow, 21, &payload, MTU).unwrap());
    let [other_a, other_b] = pair(split(FlowId::new(6).unwrap(), 20, &payload, MTU).unwrap());
    let mut output = [0; 1200];
    for (first, second) in [
        (a, a),
        (b, b),
        (a, next_b),
        (next_a, b),
        (a, other_b),
        (other_a, b),
    ] {
        assert_eq!(
            join(Some(first), Some(second), &mut output),
            Err(CodecError::MismatchedFragments)
        );
    }
    let whole = Frame {
        flow,
        fragment: None,
        payload: &payload[..10],
    };
    assert_eq!(
        join(Some(whole), Some(b), &mut output),
        Err(CodecError::MismatchedFragments)
    );
    let joined = join(Some(whole), None, &mut []).unwrap().unwrap();
    assert_eq!(joined.as_ptr(), payload.as_ptr());
}

#[test]
fn flow_varint_exact_boundaries_and_response_prefix() {
    for (value, expected) in [
        (0, &[0][..]),
        (63, &[63][..]),
        (64, &[0x40, 0x40][..]),
        (16383, &[0x7f, 0xff][..]),
    ] {
        let flow = FlowId::new(value).unwrap();
        let mut output = [0; 3];
        let len = flow.encode(&mut output).unwrap();
        assert_eq!(&output[..len], expected);
        output[len] = 99;
        assert_eq!(FlowId::decode(&output[..len + 1]), Ok((flow, len)));
        assert_eq!(
            flow.encode(&mut output[..len - 1]),
            Err(CodecError::BufferTooSmall)
        );
    }
    assert_eq!(FlowId::new(16384), Err(CodecError::InvalidFlowId));
    // QUIC allows a wider-than-minimum encoding; accepting 2 bytes does not widen the flow domain.
    assert_eq!(FlowId::decode(&[0x40, 1]), Ok((FlowId::new(1).unwrap(), 2)));
    assert_eq!(FlowId::decode(&[]), Err(CodecError::Truncated));
    assert_eq!(FlowId::decode(&[0x40]), Err(CodecError::Truncated));
    for input in [
        &[0x80][..],
        &[0x80, 0, 0x40, 0][..],
        &[0xc0, 0, 0, 0, 0, 0, 0, 0][..],
    ] {
        assert_eq!(FlowId::decode(input), Err(CodecError::InvalidFlowId));
    }
}

#[test]
fn malformed_truncated_and_invalid_frames_are_rejected() {
    let frame = Frame {
        flow: FlowId::new(64).unwrap(),
        fragment: Some(Fragment {
            packet_id: 17,
            index: FragmentIndex::Second,
        }),
        payload: &[42],
    };
    let mut output = [0; 12];
    let len = frame.encode(&mut output, 12).unwrap();
    assert_eq!(len, 12);
    for end in 0..len {
        assert!(
            Frame::decode(&output[..end]).is_err(),
            "accepted prefix of {end} bytes"
        );
    }
    for flags in 3..=255 {
        output[2] = flags;
        assert_eq!(Frame::decode(&output), Err(CodecError::InvalidFragment));
    }
    assert_eq!(
        Frame::decode(&[0x80, 0, 0, 0, 0]),
        Err(CodecError::InvalidFlowId)
    );
    assert_eq!(
        frame.encode(&mut output[..11], 12),
        Err(CodecError::BufferTooSmall)
    );
    assert_eq!(
        frame.encode_header(&mut output[..10], 12),
        Err(CodecError::BufferTooSmall)
    );
    assert_eq!(
        frame.encode_header(&mut output, 10),
        Err(CodecError::DatagramTooSmall)
    );
    assert_eq!(
        frame.encode_header(&mut output, 11),
        Err(CodecError::PacketTooLarge)
    );
    let invalid = Frame {
        payload: &[],
        ..frame
    };
    assert_eq!(
        invalid.encode_header(&mut output, 12),
        Err(CodecError::InvalidFragment)
    );
    assert_eq!(
        join(Some(invalid), None, &mut []),
        Err(CodecError::InvalidFragment)
    );
}

#[test]
fn empty_packets_and_too_small_datagram_limits() {
    let flow = FlowId::new(64).unwrap();
    assert_eq!(split(flow, 0, &[], 2), Err(CodecError::DatagramTooSmall));
    let Packet::Whole(empty) = split(flow, 0, &[], 3).unwrap() else {
        panic!("empty packet must be whole")
    };
    let mut output = [0; 3];
    assert_eq!(empty.encode(&mut output, 3), Ok(3));
    assert_eq!(Frame::decode(&output), Ok(empty));
    assert_eq!(join(Some(empty), None, &mut []).unwrap(), Some(&[][..]));
    // A complete header fits at 11 bytes, but the fragment header leaves zero payload budget.
    assert_eq!(
        split(flow, 0, &[0; 9], 11),
        Err(CodecError::DatagramTooSmall)
    );
}
