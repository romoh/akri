#!/bin/bash

GITHUB_TOKEN="b80d58f112d4293bb5e1a7f092f9357adc0b88bf"
EMAIL="romoh@microsoft.com"
USERNAME="romoh"

if [ -z "$GITHUB_TOKEN" ]; then
    echo "GITHUB_TOKEN is not defined"
    exit 1
fi

BRANCH_NAME="automated_cargo_update"

git checkout -b $BRANCH_NAME
cargo update #&& cargo test

DIFF=`git diff`
if [ -n "$DIFF" ]; then
    # configure git authorship
    git config --global user.email $EMAIL
    git config --global user.name $USERNAME

    git remote add authenticated https://romoh:$GITHUB_TOKEN@github.com/romoh/akri.git #https://github.com/romoh/akri.git

    # commit the changes to Cargo.lock
    git commit -a -m "Auto-update cargo crates"

    # push the changes so that PR API has something to compare against
    git push authenticated $BRANCH_NAME

    # finally create the PR
    curl -X POST -H "Content-Type: application/json" -H "Authorization: token $GITHUB_TOKEN" \
         --data '{"title":"Auto-update cargo crates","head":"automated_cargo_update","base":"main", "body":"Dependencies update review"}' \
         https://api.github.com/repos/romoh/akri/pulls

    # # add a remote with read/write permissions!
    # # use token authentication instead of password
    # git remote add authenticated https:/romoh/akri:$GITHUB_TOKEN@github.com/romoh/akri.git
    
    # # commit the changes to Cargo.lock
    # git commit -a -m "Auto-update cargo crates"

    # # push the changes so that PR API has something to compare against
    # git push authenticated $BRANCH_NAME

    # # finally create the PR
    # curl -X POST -H "Content-Type: application/json" -H "Authorization: token $GITHUB_TOKEN" \
    #      --data '{"title":"Auto-update cargo crates","head":"automated_cargo_update","base":"master", "body":"@atodorov review"}' \
    #      https://github.com/romoh/akri/pulls
fi