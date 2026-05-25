//@ mode: c
//@ run-status: 0

// Test flexible array member (unsized array as last field in struct)
// This is a C99 feature where the last field can be an unsized array

struct S {
    int x;
    int y;
    int arr[];  // Flexible array member - must be last field
};

// Also test with different types
struct T {
    char tag;
    int data[];  // Flexible array member
};

int main() {
    // Check that sizeof doesn't include the flexible array
    // The size should only include the sized members
    if (sizeof(struct S) != sizeof(int) * 2) {
        return 1;
    }
    
    if (sizeof(struct T) != 4) {
        return 2;
    }

    struct S s1 = {1, 2};
    
    if (s1.x != 1 || s1.y != 2) {
        return 3;
    }

    int s2_storage[] = {10, 20, 3, 15, 1000, 60, 16};

    struct S* s2 = (struct S*)&s2_storage;

    if (s2->x != 10 || s2->y != 20 || s2->arr[0] != 3 || s2->arr[4] != 16) {
        return 4;
    }

    // Flexible array members are typically used with dynamic allocation
    // For this basic test, we verify the struct can be defined
    // and sizeof works correctly
    
    return 0;
}
