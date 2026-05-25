//@ mode: c
//@ run-status: 0

void do_complex(int *p)
{
	*p = *p + 1;
}

int simple()
{
	int side_effect_counter;
	int x;

	side_effect_counter = 0;
	x = ({ int local; local = 4; do_complex(&side_effect_counter); local + 1; });
	if (x - 5)
		return 1;
	if (side_effect_counter - 1)
		return 1;

	return 0;
}

int void_type() {
    int side_effect_counter = 0;
	1 ? ({ }) : 3;
    1 ? ({ side_effect_counter += 1; int x = 5; }) : 3;
    1 ? ({ return side_effect_counter - 1; }) : 3;
}

int main() {
    if (simple())
        return 1;
    if (void_type())
        return 2;
    return 0;
}