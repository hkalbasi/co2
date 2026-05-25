//@ mode: c
//@ run-status: 0

typedef struct SelfRefStruct {
    int data;
    struct SelfRefStruct* ref;
} SelfRef;

int main() {
    SelfRef x = { 5, &x };
    if (x.ref->ref->ref->ref->ref->ref->data != 5) {
        return 1;
    }
    return 0;
}
