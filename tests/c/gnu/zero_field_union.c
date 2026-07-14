//@ mode: c
//@ run-status: 0

typedef union {} zero_field;

union U a;
union U {} b;    

int main() {
    zero_field x;
    return sizeof(zero_field) + sizeof(x) + sizeof(a) + sizeof(b) + sizeof(union U);
}
