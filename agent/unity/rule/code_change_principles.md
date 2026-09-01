## Unity C# Programming Principles

* Unity C# has a poor-performing garbage collector, so you should avoid creating and collecting temporary objects whenever possible.

* You should ensure that the runtime code you write performs well, especially logic that is called every frame, such as `Update` / `FixedUpdate`.

* Do not add features, refactor, or make “convenient improvements” beyond what the user asked for. If you discover that an improvement is necessary, report it to the user and ask for permission.

* Fixing one bug does not require cleaning up surrounding code as a side task.

* Implementing a simple feature does not require adding a large amount of extra configurability.

* Do not add docstrings, comments, or type annotations to code you did not modify.

* Add comments only when the logic itself is not self-evident.

* Do not add error handling, fallbacks, or validation for scenarios that cannot happen.

* Trust guarantees provided by internal code and the framework. Put validation only at system boundaries, such as user input and external APIs.

* When you can directly change the code, do not introduce feature flags or compatibility shims.

* Do not abstract one-off operations into helpers, utilities, or generic layers.

* Do not design for hypothetical future requirements.

* Complexity should serve only the current task: avoid both over-abstraction and leaving half-finished work.

* Three lines of similar code are usually better than premature abstraction.

* Do not use compatibility hacks, such as renaming unused variables with a leading underscore, re-exporting types that are not used, or leaving comments like `// removed` for deleted code.

* If you are sure something is no longer useful, you may delete it directly.
