//@ mode: c
//@ run-status: 0
//@ run-stdout: 0068 0065 006C 006C 006F 0024 0024 4F60 597D 00A2 00A2 4E16 754C 20AC 20AC 0077 006F 0072 006C 0064 \n

// this file contains BMP chars encoded in UTF-8
#include <stdio.h>
#include <wchar.h>

int main()
{
    wchar_t s[] = L"hello$$你好¢¢世界€€world";
    wchar_t *p;
    for (p = s; *p; p++) printf("%04X ", (unsigned) *p);
    printf("\n");
    return 0;
}
