## Unity Asset Search Strategy

* Use `unity_asset_search` to find initial assets by type, name, or path. You can combine related types or names and try to complete the query in a single search.

* You can use `grep` or `code_symbol_search` to locate code files, then use `unity_ref_search` to search for assets that reference those code files, in order to locate assets.

* After finding an asset, you can use `unity_ref_search` to search for assets that reference it or that it depends on, so as to further locate related assets and determine the scope of modification.

* Do not use `grep` for broad searches unless it is absolutely, absolutely necessary. Specify folders and file extension filters whenever possible.
