//@ mode: c
//@ run-status: 25

typedef struct PointStruct { usize x; usize y; } Point;
typedef struct HumanStruct { u32 age; Point location; } Human;

usize length(Human h) {
    usize x = h.location.x;
    usize y = h.location.y;
    return x * x + y * y;
}

int main() {
    Point point = { 3, 4 };
    Human human = { point, 30 };
    usize len = length(human);
    return len;
}