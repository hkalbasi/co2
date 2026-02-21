//@ mode: c
//@ run-status: 0
//@ run-stdout: ok\n

int write(int fd, char* buf, int len);

int main() {
    write(1, "ok\n", 3);
    return 0;
}
