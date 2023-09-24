/**
 * An anonymous struct aliased with a typedef.
 */
typedef struct {
  int foo;
} incognito;

int incognito_foo(incognito *i) {
  return __builtin_preserve_access_index(i->foo);
}
