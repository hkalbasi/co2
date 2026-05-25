//@ mode: c
//@ run-status: 0

constexpr int *global_null = (int *)0;
constexpr char global_letter = (char)65;

enum {
    generic_match = _Generic((unsigned int)1, unsigned int: 5, default: 1),
};

int matched_arr[_Generic((unsigned int)1, unsigned int: 4, default: 1)] = {};
int defaulted_arr[_Generic((double)1.0, unsigned int: 4, default: 3)] = {};
int char_cast_arr[(char)65 == 'A' ? 2 : -1] = {};

int main(void) {
    constexpr int *local_null = (int *)0;
    constexpr char local_letter = (char)66;

    if (global_null != 0 || local_null != 0) {
        return 1;
    }
    if (global_letter != 'A' || local_letter != 'B') {
        return 2;
    }
    if (sizeof(matched_arr) / sizeof(int) != 4) {
        return 3;
    }
    if (sizeof(defaulted_arr) / sizeof(int) != 3) {
        return 4;
    }
    if (sizeof(char_cast_arr) / sizeof(int) != 2) {
        return 5;
    }
    if (generic_match != 5) {
        return 6;
    }
    return 0;
}
