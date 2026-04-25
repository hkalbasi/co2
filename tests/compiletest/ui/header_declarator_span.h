typedef int Callback(int);
extern int takes(Callback cb, int x);
extern int bad(Callback cb());
         //^^^^^^^^^^^^^^^^^^ error: function returning function is not valid
