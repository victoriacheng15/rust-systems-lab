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
