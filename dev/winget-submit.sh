#!/usr/bin/env bash
# Submits winget manifests (written by dev/winget.sh) to microsoft/winget-pkgs: for each package,
# one commit with its files on a branch of the token owner's fork, and a pull request, "New
# package" the first time and "New version" after. Through GitHub's API only: winget-pkgs is far
# too large to clone. A pull request already open for the branch is left alone, so a rerun is safe;
# a package that fails does not stop the others. Needs GH_TOKEN, a token of the fork's owner that
# may push to the fork and open pull requests (a classic token with the public_repo scope).
# WINGET_DRY_RUN=1 prints the requests instead of sending them (for the tests).
#
# Usage: dev/winget-submit.sh MANIFESTS_DIR VERSION PACKAGE_ID...
set -euo pipefail

manifests=$1
version=$2
shift 2
upstream=microsoft/winget-pkgs

api() {
    if [[ -n ${WINGET_DRY_RUN:-} ]]; then
        echo "api $*" >&2
        # Only a request with a body (--input -) reads it.
        local body=""
        [[ " $* " == *" --input - "* ]] && body=$(cat)
        case "$*" in
            "user --jq .login") echo tester ;;
            *"/git/ref/heads/master --jq .object.sha") echo base-commit ;;
            *"/git/commits/base-commit --jq .tree.sha") echo base-tree ;;
            *"/git/trees --input - --jq .sha") printf '%s' "$body" > "${WINGET_DRY_RUN_TREE:-/dev/null}"; echo new-tree ;;
            *"/git/commits --input - --jq .sha") echo new-commit ;;
            *"/contents/manifests/k/Kankaj/"*) [[ -n ${WINGET_DRY_RUN_EXISTS:-} ]] || return 1 ;;
            *"pulls?head="*) echo 0 ;;
            *) ;;
        esac
        return 0
    fi
    gh api "$@"
}

owner=$(api user --jq .login)
fork="$owner/winget-pkgs"
api -X POST "repos/$fork/merge-upstream" -f branch=master > /dev/null
base=$(api "repos/$upstream/git/ref/heads/master" --jq .object.sha)
base_tree=$(api "repos/$upstream/git/commits/$base" --jq .tree.sha)

submit() {
    local id=$1
    local path="manifests/k/Kankaj/${id#Kankaj.}"
    local dir="$manifests/$path/$version"
    local branch="$id-$version"
    [[ -d $dir ]] || { echo "winget-submit: no manifests in $dir" >&2; return 1; }

    if [[ $(api "repos/$upstream/pulls?head=$owner:$branch&state=open" --jq length) != 0 ]]; then
        echo "winget-submit: $id $version: its pull request is open already"
        return 0
    fi
    local kind="New version"
    api "repos/$upstream/contents/$path" > /dev/null 2>&1 || kind="New package"

    # One tree on top of winget-pkgs' master with the package's files, one commit, one branch.
    local tree commit
    tree=$(for file in "$dir"/*.yaml; do
            jq -n --arg path "${file#"$manifests"/}" --rawfile content "$file" \
                '{path: $path, mode: "100644", type: "blob", content: $content}'
        done | jq -s --arg base "$base_tree" '{base_tree: $base, tree: .}' \
        | api -X POST "repos/$fork/git/trees" --input - --jq .sha)
    commit=$(jq -n --arg message "$kind: $id version $version" --arg tree "$tree" --arg parent "$base" \
        '{message: $message, tree: $tree, parents: [$parent]}' \
        | api -X POST "repos/$fork/git/commits" --input - --jq .sha)
    api -X POST "repos/$fork/git/refs" -f "ref=refs/heads/$branch" -f "sha=$commit" > /dev/null 2>&1 \
        || api -X PATCH "repos/$fork/git/refs/heads/$branch" -f "sha=$commit" -F force=true > /dev/null

    local body="Senzor $version, from its release workflow (https://github.com/kankaj/Senzor/releases/tag/v$version)."
    api -X POST "repos/$upstream/pulls" -f "title=$kind: $id version $version" \
        -f "head=$owner:$branch" -f base=master -f "body=$body" --jq .html_url
}

failed=0
for id in "$@"; do
    submit "$id" || { echo "winget-submit: $id $version could not be submitted" >&2; failed=1; }
done
exit "$failed"
