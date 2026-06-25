//@ mode: c
//@ run-status: 0
//@ run-stdout: foo\nbar\n--\nfoo\nbar\n--\n__x86_64__\n__x86_64\n__amd64__\n--\ngood\nbye\nworld\n--\n

#include <stdint.h>
#include <stdio.h>
#include <string.h>

const char str[] = 
    "foo\0"
    "bar\0";


const char * const target_machine_defs =
    "__x86_64__\0"
    "__x86_64\0"
    "__amd64__\0";

void print_double_null_strings(const char* p) {
    for (int i = 0; ; i++) {
        puts(p);
        p = strchr(p, 0) + 1;
        if (*p == 0)
            break;
    }
    puts("--");
}

int main(void) {
    if (sizeof("foo\0bar\0") != 9) {
        return 1;
    }
    if (sizeof("foo\0" "bar\0") != 9) {
        return 1;
    }
    if (sizeof(str) != 9) {
        return 1;
    }
    print_double_null_strings("foo\0bar\0");
    print_double_null_strings(str);
    print_double_null_strings(target_machine_defs);
    print_double_null_strings("good\\
nbye\0world\0");
    return 0;
}
