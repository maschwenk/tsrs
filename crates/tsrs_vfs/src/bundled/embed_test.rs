use super::{EmbeddedFile, EMBEDDED_CONTENTS};

#[test]
fn concurrent_first_reads_share_text() {
    let (_, source) = EMBEDDED_CONTENTS.iter().find(|(name, _)| *name == "libs/lib.dom.d.ts").unwrap();
    let file = EmbeddedFile::new(source.compressed, source.size);
    let ready = std::sync::Barrier::new(8);
    std::thread::scope(|scope| {
        let mut readers: Vec<_> = std::iter::repeat_with(|| {
            scope.spawn(|| {
                ready.wait();
                file.contents()
            })
        }).take(8).collect();
        let first = readers.pop().unwrap().join().unwrap();
        assert_eq!(first, include_str!("../../libs/lib.dom.d.ts"));
        for reader in readers {
            assert!(std::ptr::eq(first, reader.join().unwrap()));
        }
    });
}
