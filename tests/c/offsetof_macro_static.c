//@ mode: c
//@ run-status: 0

typedef unsigned long size_t;
#define offsetof(type, field) ((size_t) &((type *)0)->field)

typedef int Getter(void);

typedef struct Point {
    int x;
    int y;
} Point;

typedef struct Entry {
    const char *name;
    Getter *getter;
    size_t magic;
} Entry;

int get_y(void) {
    return 0;
}

static const Entry entries[] = {
    { "y", get_y, offsetof(Point, y) },
};

int main(void) {
    return entries[0].magic == sizeof(int) ? 0 : 1;
}
