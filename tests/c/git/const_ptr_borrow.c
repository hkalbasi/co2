//@ mode: c

/*
 * Reproduction of "cannot borrow data in a *const pointer as mutable" 
 * found in git's attr.c.
 */

struct git_attr {
    unsigned int attr_nr;
    char name[1]; // FLEX_ARRAY
};

struct all_attrs_item {
    const struct git_attr *attr;
    const char *value;
};

struct attr_check {
    int all_attrs_nr;
    struct all_attrs_item *all_attrs;
};

void git_all_attrs(struct attr_check *check) {
    int i;
    for (i = 0; i < check->all_attrs_nr; i++) {
        // This pattern caused E0596 in co2cc
        const char *name = check->all_attrs[i].attr->name;
        if (name[0] == 't') return;
    }
}

int main() {
    struct git_attr a = {1, "t"};
    struct all_attrs_item it = {&a, "val"};
    struct attr_check c = {1, &it};
    git_all_attrs(&c);
    return 0;
}
