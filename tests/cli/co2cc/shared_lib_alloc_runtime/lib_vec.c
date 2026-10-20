int sum_vector(int n) {
    std::vec::Vec::<i32> values = std::vec::Vec::<i32>::new();
    for (int i = 0; i < n; i++) {
        values.push(i + 1);
    }
    int sum = 0;
    for (int i = 0; i < n; i++) {
        sum += *values.get_unchecked::<usize>(i);
    }
    return sum;
}
