//@ mode: c
//@ run-status: 0
//@ run-stdout: sizeof=24 off_class=6 off_prev=8 off_next=16

#include <stdio.h>
#include <stddef.h>
#include <stdint.h>

typedef struct Header {
    int ref_count;
    uint8_t a : 7;
    uint8_t b : 1;
    uint8_t c : 1;
    uint8_t d : 1;
    uint8_t e : 1;
    uint8_t f : 1;
    uint8_t g : 1;
    uint8_t h : 1;
    uint8_t i : 1;
    uint8_t j : 1;
    uint16_t class_id;
    void *link_prev;
    void *link_next;
} Header;

int main(void) {
    Header h = {12, 3, 1, 0, 1};

    if (h.ref_count != 12 || h.a != 3 || h.b != 1 || h.c != 0 || h.d != 1) {
        return 1;
    }

    printf("sizeof=%zu off_class=%zu off_prev=%zu off_next=%zu\n",
           sizeof(Header), offsetof(Header, class_id), offsetof(Header, link_prev), offsetof(Header, link_next));
    return 0;
}
