double sqlite3_magic(double value);

int main(void) {
    return sqlite3_magic(2.5) != 5.0;
}
