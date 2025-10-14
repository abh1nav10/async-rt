#[cfg(test)]
mod stack_test {
    use electron::Stack;
    #[test]
    fn test_one() {
        let new = &Stack::new();
        std::thread::scope(|s| {
            for i in 0..500 {
                s.spawn(move || {
                    let _ = new.insert(i);
                });
            }
        });
        std::thread::scope(|s| {
            for _ in 0..500 {
                s.spawn(move || {
                    let _ = new.delete();
                });
            }
        });
    }
}
