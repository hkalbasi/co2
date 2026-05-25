//@ mode: c
//@ run-status: 240

struct { int x; int y; } a, b;
struct { int x, y; } c;
struct { int z; union { int x, y; }; } d;
struct { int *z; struct { int x, y; }; } e = { .x = 100, .y = 200, .z = 0 };

typedef struct { int a, b; } S1;

int main() {
    S1 ar[3] = {1, 2, 3, 4, 5, 6};
    if (ar[1].b != 4 || ar[0].a != 1) {
        return 6;
    }

    a.x = 2;
    a.y = 3;
    b = a;
    c.y = a.x;
    d.z = 10;
    d.x = 20;

    int ar2[3] = {10, 20, 3};
    e.z = ar2;

    return a.x + b.y + c.y + d.y + d.z + e.y + e.z[2];
}
