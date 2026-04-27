use super::*;

#[test]
fn parses_single_file_torrent_metadata() {
    let torrent =
        b"d8:announce32:https://tracker.example/announce4:infod6:lengthi12e4:name8:test.txt12:piece lengthi16384e6:pieces20:aaaaaaaaaaaaaaaaaaaaee";

    let meta = TorrentMeta::from_bytes(torrent).expect("torrent should parse");

    assert_eq!(
        meta.announce.as_deref(),
        Some("https://tracker.example/announce")
    );
    assert_eq!(meta.name, "test.txt");
    assert_eq!(meta.piece_length, 16384);
    assert_eq!(meta.total_length, Some(12));
    assert_eq!(meta.file_count, None);
    assert_eq!(meta.piece_count, 1);
}

#[test]
fn hashes_original_info_bytes() {
    let info =
        b"d6:lengthi12e4:name8:test.txt12:piece lengthi16384e6:pieces20:aaaaaaaaaaaaaaaaaaaae";
    let torrent =
        b"d8:announce32:https://tracker.example/announce4:infod6:lengthi12e4:name8:test.txt12:piece lengthi16384e6:pieces20:aaaaaaaaaaaaaaaaaaaaee";

    let meta = TorrentMeta::from_bytes(torrent).expect("torrent should parse");

    assert_eq!(meta.info_hash_hex, sha1_hex(info));
}

#[test]
fn parses_multi_file_marker() {
    let torrent = b"d4:infod5:filesld6:lengthi5e4:pathl5:a.txteed6:lengthi7e4:pathl5:b.txteee4:name4:pack12:piece lengthi32768e6:pieces20:bbbbbbbbbbbbbbbbbbbbee";

    let meta = TorrentMeta::from_bytes(torrent).expect("torrent should parse");

    assert_eq!(meta.mode_label(), "multi-file");
    assert_eq!(meta.total_length, None);
    assert_eq!(meta.file_count, Some(2));
}

#[test]
fn sha1_matches_known_vector() {
    assert_eq!(sha1_hex(b"abc"), "a9993e364706816aba3e25717850c26c9cd0d89d");
}

#[test]
fn percent_encodes_binary_tracker_parameters() {
    assert_eq!(percent_encode_bytes(&[0, b'A', b'-', 255]), "%00A-%FF");
}

#[test]
fn builds_tracker_url_with_required_query_fields() {
    let torrent =
        b"d8:announce32:https://tracker.example/announce4:infod6:lengthi12e4:name8:test.txt12:piece lengthi16384e6:pieces20:aaaaaaaaaaaaaaaaaaaaee";
    let meta = TorrentMeta::from_bytes(torrent).expect("torrent should parse");
    let url =
        build_tracker_url(&meta, b"-RS0001-000000000001", 6881).expect("tracker URL should build");

    assert!(url.starts_with("https://tracker.example/announce?"));
    assert!(url.contains("info_hash="));
    assert!(url.contains("peer_id=-RS0001-000000000001"));
    assert!(url.contains("port=6881"));
    assert!(url.contains("uploaded=0"));
    assert!(url.contains("downloaded=0"));
    assert!(url.contains("left=12"));
    assert!(url.contains("compact=1"));
    assert!(url.contains("event=started"));
}

#[test]
fn parses_compact_tracker_peers() {
    let response = b"d8:intervali1800e5:peers12:\x7f\x00\x00\x01\x1a\xe1\xc0\x00\x02\x05\xc8\x1ae";

    let response = TrackerResponse::from_bytes(response).expect("response should parse");

    assert_eq!(response.interval, Some(1800));
    assert_eq!(
        response.peers,
        vec![
            SocketAddrV4::new(Ipv4Addr::new(127, 0, 0, 1), 6881),
            SocketAddrV4::new(Ipv4Addr::new(192, 0, 2, 5), 51226),
        ]
    );
}

#[test]
fn reports_tracker_failure_reason() {
    let response = b"d14:failure reason13:bad info hashe";

    let error = TrackerResponse::from_bytes(response).expect_err("response should fail");

    assert!(error.to_string().contains("bad info hash"));
}

#[test]
fn builds_and_parses_peer_handshake() {
    let info_hash = [1u8; 20];
    let peer_id = *b"-RS0001-000000000001";

    let bytes = build_peer_handshake(&info_hash, &peer_id);
    let parsed = parse_peer_handshake(&bytes).expect("handshake should parse");

    assert_eq!(bytes[0], 19);
    assert_eq!(&bytes[1..20], b"BitTorrent protocol");
    assert_eq!(parsed.reserved, [0u8; 8]);
    assert_eq!(parsed.info_hash, info_hash);
    assert_eq!(parsed.peer_id, peer_id);
}

#[test]
fn rejects_peer_handshake_with_wrong_protocol() {
    let mut bytes = build_peer_handshake(&[1u8; 20], b"-RS0001-000000000001");
    bytes[1] = b'X';

    let error = parse_peer_handshake(&bytes).expect_err("handshake should fail");

    assert!(error.to_string().contains("BitTorrent protocol"));
}

#[tokio::test]
async fn performs_peer_handshake_over_stream() {
    let (mut client, mut server_stream) = tokio::io::duplex(256);
    let info_hash = [3u8; 20];
    let remote_peer_id = *b"-RS0001-REMOTE000001";

    let server = tokio::spawn(async move {
        let mut request = [0u8; 68];
        server_stream
            .read_exact(&mut request)
            .await
            .expect("request should read");
        let request = parse_peer_handshake(&request).expect("request should parse");
        let response = build_peer_handshake(&request.info_hash, &remote_peer_id);
        server_stream
            .write_all(&response)
            .await
            .expect("response should write");
    });

    let response = perform_peer_handshake(&mut client, &info_hash, b"-RS0001-000000000001")
        .await
        .expect("handshake should complete");

    server.await.expect("server task should complete");
    assert_eq!(response.info_hash, info_hash);
    assert_eq!(response.peer_id, remote_peer_id);
}

#[tokio::test]
async fn reads_peer_message_from_stream() {
    let (mut client, mut server_stream) = tokio::io::duplex(256);
    let frame = PeerMessage::Bitfield(vec![0b1010_0000, 0b0100_0000]).encode();

    let server = tokio::spawn(async move {
        server_stream
            .write_all(&frame)
            .await
            .expect("message should write");
    });

    let message = read_peer_message(&mut client)
        .await
        .expect("message should read");

    server.await.expect("server task should complete");
    assert_eq!(
        message,
        PeerMessage::Bitfield(vec![0b1010_0000, 0b0100_0000])
    );
    assert_eq!(message.summary(), "bitfield (2 bytes)");
}

#[test]
fn encodes_and_decodes_keep_alive_message() {
    let frame = PeerMessage::KeepAlive.encode();

    assert_eq!(frame, vec![0, 0, 0, 0]);
    assert_eq!(
        PeerMessage::decode(&frame).expect("keep-alive should decode"),
        PeerMessage::KeepAlive
    );
}

#[test]
fn encodes_and_decodes_simple_peer_messages() {
    let cases = [
        (PeerMessage::Choke, vec![0, 0, 0, 1, 0]),
        (PeerMessage::Unchoke, vec![0, 0, 0, 1, 1]),
        (PeerMessage::Interested, vec![0, 0, 0, 1, 2]),
        (PeerMessage::NotInterested, vec![0, 0, 0, 1, 3]),
    ];

    for (message, expected_frame) in cases {
        let frame = message.encode();

        assert_eq!(frame, expected_frame);
        assert_eq!(
            PeerMessage::decode(&frame).expect("message should decode"),
            message
        );
    }
}

#[test]
fn encodes_and_decodes_payload_peer_messages() {
    let cases = [
        PeerMessage::Have(7),
        PeerMessage::Bitfield(vec![0b1010_0000, 0b0100_0000]),
        PeerMessage::Request {
            index: 2,
            begin: 16_384,
            length: 16_384,
        },
        PeerMessage::Piece {
            index: 2,
            begin: 16_384,
            block: vec![1, 2, 3, 4],
        },
        PeerMessage::Cancel {
            index: 2,
            begin: 16_384,
            length: 16_384,
        },
        PeerMessage::Port(6881),
    ];

    for message in cases {
        let frame = message.encode();

        assert_eq!(
            PeerMessage::decode(&frame).expect("message should decode"),
            message
        );
    }
}

#[test]
fn rejects_peer_message_with_bad_length_prefix() {
    let error = PeerMessage::decode(&[0, 0, 0, 5, 2]).expect_err("frame length should be rejected");

    assert!(error.to_string().contains("length exceeds frame"));
}

#[test]
fn rejects_peer_message_with_wrong_payload_size() {
    let error =
        PeerMessage::decode(&[0, 0, 0, 2, 4, 0]).expect_err("have payload should be rejected");

    assert!(error.to_string().contains("have message payload"));
}

#[test]
fn peer_state_tracks_choke_and_interest_messages() {
    let mut state = PeerState::new(4);

    state.apply_inbound(&PeerMessage::Unchoke).expect("unchoke");
    state
        .apply_inbound(&PeerMessage::Interested)
        .expect("interested");
    state
        .apply_outbound(&PeerMessage::Interested)
        .expect("client interested");

    assert!(!state.peer_choking);
    assert!(state.peer_interested);
    assert!(state.client_interested);

    state.apply_inbound(&PeerMessage::Choke).expect("choke");
    state
        .apply_outbound(&PeerMessage::NotInterested)
        .expect("client not interested");

    assert!(state.peer_choking);
    assert!(!state.client_interested);
}

#[test]
fn peer_state_tracks_bitfield_and_have_messages() {
    let mut state = PeerState::new(10);

    state
        .apply_inbound(&PeerMessage::Bitfield(vec![0b1010_0000, 0b0100_0000]))
        .expect("bitfield should apply");

    assert!(state.has_piece(0));
    assert!(!state.has_piece(1));
    assert!(state.has_piece(2));
    assert!(!state.has_piece(3));
    assert!(!state.has_piece(8));
    assert!(state.has_piece(9));
    assert_eq!(state.available_piece_count(), 3);

    state.apply_inbound(&PeerMessage::Have(1)).expect("have");

    assert!(state.has_piece(1));
    assert_eq!(state.available_piece_count(), 4);
}

#[test]
fn peer_state_rejects_out_of_range_piece_availability() {
    let mut state = PeerState::new(2);

    let error = state
        .apply_inbound(&PeerMessage::Have(3))
        .expect_err("piece index should fail");

    assert!(error.to_string().contains("outside torrent piece count"));
}

#[test]
fn peer_state_tracks_outbound_requests_and_cancels() {
    let mut state = PeerState::new(4);
    let request = PeerMessage::Request {
        index: 2,
        begin: 0,
        length: 16_384,
    };

    state.apply_outbound(&request).expect("request");

    assert_eq!(
        state.requested_blocks,
        vec![BlockRequest {
            index: 2,
            begin: 0,
            length: 16_384,
        }]
    );

    state
        .apply_outbound(&PeerMessage::Cancel {
            index: 2,
            begin: 0,
            length: 16_384,
        })
        .expect("cancel");

    assert!(state.requested_blocks.is_empty());
}

#[test]
fn peer_state_tracks_inbound_piece_blocks_and_peer_requests() {
    let mut state = PeerState::new(4);

    state
        .apply_inbound(&PeerMessage::Request {
            index: 1,
            begin: 32,
            length: 16,
        })
        .expect("peer request");
    state
        .apply_inbound(&PeerMessage::Piece {
            index: 2,
            begin: 64,
            block: vec![1, 2, 3],
        })
        .expect("piece");

    assert_eq!(
        state.peer_requested_blocks,
        vec![BlockRequest {
            index: 1,
            begin: 32,
            length: 16,
        }]
    );
    assert_eq!(
        state.received_blocks,
        vec![PieceBlock {
            index: 2,
            begin: 64,
            block: vec![1, 2, 3],
        }]
    );

    state
        .apply_inbound(&PeerMessage::Cancel {
            index: 1,
            begin: 32,
            length: 16,
        })
        .expect("cancel peer request");

    assert!(state.peer_requested_blocks.is_empty());
}

#[test]
fn computes_piece_lengths_for_single_file_torrents() {
    let torrent =
        b"d4:infod6:lengthi40000e4:name8:test.bin12:piece lengthi16384e6:pieces60:aaaaaaaaaaaaaaaaaaaabbbbbbbbbbbbbbbbbbbbccccccccccccccccccccee";
    let meta = TorrentMeta::from_bytes(torrent).expect("torrent should parse");

    assert_eq!(meta.piece_length_at(0).expect("piece 0"), 16_384);
    assert_eq!(meta.piece_length_at(1).expect("piece 1"), 16_384);
    assert_eq!(meta.piece_length_at(2).expect("piece 2"), 7_232);
}

#[test]
fn builds_piece_requests_in_standard_block_sizes() {
    let requests = build_piece_requests(3, 40_000).expect("requests should build");

    assert_eq!(
        requests,
        vec![
            BlockRequest {
                index: 3,
                begin: 0,
                length: 16_384,
            },
            BlockRequest {
                index: 3,
                begin: 16_384,
                length: 16_384,
            },
            BlockRequest {
                index: 3,
                begin: 32_768,
                length: 7_232,
            },
        ]
    );
}

#[tokio::test]
async fn downloads_piece_blocks_with_bounded_pipeline() {
    let (mut client, mut server) = tokio::io::duplex(128 * 1024);
    let piece_length = 40_000usize;
    let pipeline = 2usize;

    let server_task = tokio::spawn(async move {
        let interested = read_peer_message(&mut server)
            .await
            .expect("interested should read");
        assert_eq!(interested, PeerMessage::Interested);

        write_peer_message(&mut server, &PeerMessage::Bitfield(vec![0b1000_0000]))
            .await
            .expect("bitfield should write");
        write_peer_message(&mut server, &PeerMessage::Unchoke)
            .await
            .expect("unchoke should write");

        let first = read_peer_message(&mut server)
            .await
            .expect("first request should read");
        let second = read_peer_message(&mut server)
            .await
            .expect("second request should read");

        let first = match first {
            PeerMessage::Request {
                index,
                begin,
                length,
            } => BlockRequest {
                index,
                begin,
                length,
            },
            other => panic!("expected request, got {other:?}"),
        };
        let second = match second {
            PeerMessage::Request {
                index,
                begin,
                length,
            } => BlockRequest {
                index,
                begin,
                length,
            },
            other => panic!("expected request, got {other:?}"),
        };

        assert_eq!(first.begin, 0);
        assert_eq!(second.begin, 16_384);

        write_peer_message(
            &mut server,
            &PeerMessage::Piece {
                index: first.index,
                begin: first.begin,
                block: vec![1; first.length as usize],
            },
        )
        .await
        .expect("first piece should write");

        let third = read_peer_message(&mut server)
            .await
            .expect("third request should read after one block returns");
        let third = match third {
            PeerMessage::Request {
                index,
                begin,
                length,
            } => BlockRequest {
                index,
                begin,
                length,
            },
            other => panic!("expected request, got {other:?}"),
        };

        assert_eq!(third.begin, 32_768);

        for request in [second, third] {
            write_peer_message(
                &mut server,
                &PeerMessage::Piece {
                    index: request.index,
                    begin: request.begin,
                    block: vec![(request.begin / 16_384 + 1) as u8; request.length as usize],
                },
            )
            .await
            .expect("piece should write");
        }
    });

    let piece = download_piece_from_peer(&mut client, 1, 0, piece_length, pipeline)
        .await
        .expect("piece should download");

    server_task.await.expect("server task should complete");
    assert_eq!(piece.len(), piece_length);
    assert_eq!(&piece[0..16_384], vec![1; 16_384].as_slice());
    assert_eq!(&piece[16_384..32_768], vec![2; 16_384].as_slice());
    assert_eq!(&piece[32_768..40_000], vec![3; 7_232].as_slice());
}
