int main(void) {
    long sum = 0;
    for (long i = 0; i < 50000000; i++) {
        sum += i;
    }
    return (int)(sum % 256);
}
