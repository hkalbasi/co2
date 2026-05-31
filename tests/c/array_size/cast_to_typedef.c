//@ mode: c
//@ run-status: 0

typedef unsigned int u32;

int arr[(u32)4];

int main(void) {
    return (int)(sizeof(arr) / sizeof(arr[0]) != 4);
}
