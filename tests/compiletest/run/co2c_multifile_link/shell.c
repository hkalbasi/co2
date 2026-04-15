double sqlite3_magic(double value);

static double common_function() {
    return 5.0;
}

int main(void) {
    return sqlite3_magic(2.6) != common_function();
}
