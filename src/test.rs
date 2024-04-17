pub struct Tests<T> {
    pub cases: Vec<Test<T>>,
}

pub struct Test<T> {
    pub input: String,
    pub expect: T,
}

impl<T> Tests<T> {
    pub fn new() -> Self {
        Tests { cases: Vec::new() }
    }
    pub fn add(&mut self, case: (&str, T)) {
        self.cases.push(Test {
            input: case.0.to_string(),
            expect: case.1,
        })
    }
}
