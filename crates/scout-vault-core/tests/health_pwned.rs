//! Leak check against a fake local server: checks that only the
//! 5-character hash prefix goes out in the request, that each prefix is
//! queried once and that padding (count 0) doesn't count.

use scout_vault_core::health::{check_pwned, pwned_hash};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

#[tokio::test]
async fn only_prefixes_leave_and_counts_come_back() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let (_, leaked_suffix) = pwned_hash("password");
    let (_, safe_suffix) = pwned_hash("k#9Lq2!vZp8@xR-único");
    let server = tokio::spawn(async move {
        let mut paths = Vec::new();
        for _ in 0..2 {
            let (mut sock, _) = listener.accept().await.unwrap();
            let mut buf = vec![0u8; 4096];
            let n = sock.read(&mut buf).await.unwrap();
            let request = String::from_utf8_lossy(&buf[..n]).to_string();
            let path = request.split_whitespace().nth(1).unwrap().to_owned();
            assert!(
                request.to_ascii_lowercase().contains("add-padding: true"),
                "pede enchimento"
            );
            let body = format!(
                "{leaked_suffix}:42\r\n{safe_suffix}:0\r\n00000000000000000000000000000000000:3"
            );
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            sock.write_all(response.as_bytes()).await.unwrap();
            paths.push(path);
        }
        paths
    });

    let found = check_pwned(
        &format!("http://{addr}"),
        &[
            ("vazada".to_owned(), "password"),
            ("repetida".to_owned(), "password"),
            ("segura".to_owned(), "k#9Lq2!vZp8@xR-único"),
        ],
    )
    .await
    .unwrap();
    assert_eq!(
        found,
        [("repetida".to_owned(), 42), ("vazada".to_owned(), 42)]
    );

    let mut paths = server.await.unwrap();
    paths.sort();
    let mut expected = vec![
        format!("/range/{}", pwned_hash("password").0),
        format!("/range/{}", pwned_hash("k#9Lq2!vZp8@xR-único").0),
    ];
    expected.sort();
    assert_eq!(paths, expected, "dois prefixos, cada um uma vez");
    for p in &paths {
        assert_eq!(p.len(), "/range/".len() + 5, "só 5 caracteres do hash");
    }
}
