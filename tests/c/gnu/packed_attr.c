//@ mode: c
//@ run-status: 0

#include <stddef.h>
#include <assert.h>

// Attribute after the closing brace packs the defined struct.
struct trailing {
    char a;
    int x;
} __attribute__((packed));

// Attribute between the keyword and the tag.
struct __attribute__((packed)) leading {
    char a;
    int x;
};

// Typedef of an anonymous packed struct.
typedef struct {
    char a;
    int x;
} __attribute__((packed)) anon_t;

// Packed union: minimal alignment.
union __attribute__((packed)) pun {
    char c;
    int x;
};

_Static_assert(offsetof(struct trailing, x) == 1, "trailing pack offset");
_Static_assert(sizeof(struct trailing) == 5, "trailing pack size");
_Static_assert(offsetof(struct leading, x) == 1, "leading pack offset");
_Static_assert(sizeof(struct leading) == 5, "leading pack size");
_Static_assert(sizeof(anon_t) == 5, "anon pack size");
_Static_assert(sizeof(union pun) == 4, "packed union size");
_Static_assert(_Alignof(anon_t) == 1, "packed align");

void inner() {
    typedef struct {
        char a;
        int x;
    } __attribute__((packed)) inner;
    _Static_assert(sizeof(inner) == 5, "inner pack size");
    _Static_assert(_Alignof(inner) == 1, "inner packed align");
    assert(sizeof(inner) == 5);
    assert(_Alignof(inner) == 1);
}

int main() {
    struct trailing t = { 'A', 0x12345678 };
    if (t.a != 'A' || t.x != 0x12345678) return 1;
    struct leading l = { 'B', 0x11223344 };
    if (l.a != 'B' || l.x != 0x11223344) return 2;
    anon_t a = { 'C', 10 };
    if (a.a != 'C' || a.x != 10) return 3;
    // Byte-exact layout: x must directly follow a.
    unsigned char *bytes = (unsigned char *)&t;
    if (bytes[0] != 'A' || bytes[1] != 0x78 || bytes[4] != 0x12) return 4;
    // Unpacked structs are unaffected.
    struct { char q; int w; } plain;
    if (sizeof(plain) != 8 || offsetof(typeof(plain), w) != 4) return 5;
    union pun u;
    u.x = 0x01020304;
    if (u.c != 0x04 && u.c != (char)0x04) return 6;
    inner();
    return 0;
}
