use super::*;
use std::{
    io::{Read, Write},
    net::TcpListener,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    thread,
    time::{Duration, Instant},
};

#[test]
fn downloads_overlap_without_exceeding_the_parallel_bound() {
    tokio::runtime::Runtime::new().unwrap().block_on(async {
        const FILE_COUNT: usize = 64;
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let address = listener.local_addr().unwrap();
        let active = Arc::new(AtomicUsize::new(0));
        let peak = Arc::new(AtomicUsize::new(0));
        let observed = peak.clone();
        let server = thread::spawn(move || {
            let mut replies = Vec::new();
            let deadline = Instant::now() + Duration::from_secs(10);
            while replies.len() < FILE_COUNT && Instant::now() < deadline {
                let Ok((mut socket, _)) = listener.accept() else {
                    thread::sleep(Duration::from_millis(5));
                    continue;
                };
                socket.set_nonblocking(false).unwrap();
                socket.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
                let active = active.clone();
                let peak = peak.clone();
                replies.push(thread::spawn(move || {
                    let mut request = [0; 4096];
                    socket.read(&mut request).unwrap();
                    let count = active.fetch_add(1, Ordering::SeqCst) + 1;
                    peak.fetch_max(count, Ordering::SeqCst);
                    thread::sleep(Duration::from_millis(100));
                    socket.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 8\r\nConnection: close\r\n\r\nexpected").unwrap();
                    active.fetch_sub(1, Ordering::SeqCst);
                }));
            }
            assert_eq!(replies.len(), FILE_COUNT);
            for reply in replies { reply.join().unwrap(); }
        });
        let dir = std::env::temp_dir().join(format!("launcher-parallel-{}", std::process::id()));
        let entries: Vec<_> = (0..FILE_COUNT).map(|i| FileEntry { path: format!("textures/{i}.blp"), sha256: hex::encode(Sha256::digest(b"expected")), size: 8 }).collect();
        let mut done = Vec::new();
        download_files(&reqwest::Client::new(), &format!("http://{address}"), &dir, &entries, |entry, count| { done.push((entry.path.clone(), count)); Ok(()) }).await.unwrap();
        server.join().unwrap();
        assert!(
            observed.load(Ordering::SeqCst) >= 8,
            "independent downloads remain request-latency-bound"
        );
        assert!(observed.load(Ordering::SeqCst) <= MAX_PARALLEL_DOWNLOADS);
        assert_eq!(done.len(), FILE_COUNT);
        for entry in entries { assert_eq!(std::fs::read(dir.join(entry.path)).unwrap(), b"expected"); }
        std::fs::remove_dir_all(dir).unwrap();
    });
}
