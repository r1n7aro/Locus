"""qrenderdoc-hosted worker for the importable locus_renderdoc facade."""

import os


def main():
    request_path = os.environ["LOCUS_RENDERDOC_REQUEST"]
    response_path = os.environ["LOCUS_RENDERDOC_RESPONSE"]
    response = {"ok": False}
    try:
        import pickle
        import sys
        import traceback

        scripts_dir = os.environ["LOCUS_RENDERDOC_MODULE_ROOT"]
        if scripts_dir not in sys.path:
            sys.path.insert(0, scripts_dir)
        import locus_renderdoc

        with open(request_path, "rb") as handle:
            request = pickle.load(handle)
        response["value"] = locus_renderdoc._worker_dispatch(
            request["method"],
            request.get("arguments", {}),
        )
        response["ok"] = True
    except BaseException as error:
        response["error"] = "{}: {}".format(type(error).__name__, error)
        try:
            response["traceback"] = traceback.format_exc()
        except Exception:
            pass

    import pickle
    temporary = response_path + ".tmp"
    try:
        with open(temporary, "wb") as handle:
            pickle.dump(response, handle, protocol=4)
    except Exception as error:
        response = {
            "ok": False,
            "error": "Response serialization failed: {}: {}".format(
                type(error).__name__, error
            ),
        }
        with open(temporary, "wb") as handle:
            pickle.dump(response, handle, protocol=4)
    os.replace(temporary, response_path)
    return 0 if response["ok"] else 1


raise SystemExit(main())
