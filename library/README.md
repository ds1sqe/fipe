# Math and statistics source modules

`math.fipe` and `stats.fipe` are ordinary Fipe file modules. Copy the modules
you use beside your application's main script, then import them with `use math`
or `use stats`. Imports resolve beside that script, even when you run it from
another directory. Installing `fipe-cli` installs the command; it does not install
these source files into a global module directory.

From a checkout or extracted `fipe` source archive, with the `fipe` command
available, copy and run the examples:

```sh
mkdir math-example
cp library/math.fipe library/stats.fipe library/summary.fipe library/interpolate.fipe math-example/
fipe check math-example/summary.fipe math-example/interpolate.fipe
fipe run math-example/summary.fipe
fipe run math-example/interpolate.fipe
```

The summary prints:

```text
(Just 5.0, Just 4.0, Just 2.0)
```

The interpolation and dot-product example prints:

```text
(12.5, Just 11.0)
```

`math.lerp` interpolates or extrapolates between two values. `math.dot` returns
`Nothing` for unequal vector lengths and `Just 0.0` for two empty vectors.

`stats.mean`, `stats.variance` and `stats.stddev` return `Nothing` for empty
input. Population variance divides by the number of values; sample variance
divides by one fewer. `stats.sample_variance` and `stats.sample_stddev` require
at least two values and otherwise return `Nothing`. The modules' public source
comments describe these results at each function.

All operations use F64 arithmetic. A nonfinite intermediate result Faults even
when the mathematical final result would be finite; there is no automatic
arbitrary-precision fallback. Equal interpolation endpoints remain constant.

The source archive contains these modules and both examples under `library/`.
They require no optional native provider or numerical engine. See the
[development guide](../Documentation/development.md) to build the command from
the checkout, and the [examples index](../Documentation/examples/README.md) for
other runnable scripts.
