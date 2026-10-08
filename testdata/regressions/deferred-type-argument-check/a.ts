// Checking this file first computes the type of `make`, which checks the signature of b.ts's arrow function, including
// the constraint of `Box<number>`. That check's error belongs to b.ts and must still be reported once b.ts is checked.
const made = make();
