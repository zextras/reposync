
#!/bin/bash

# this scripts update the project to reflect changes in the openapi specs

VERSION="5.1.0"
[ -f "openapi-generator-cli-${VERSION}.jar" ] || wget "https://repo1.maven.org/maven2/org/openapitools/openapi-generator-cli/${VERSION}/openapi-generator-cli-${VERSION}.jar"

java -jar "openapi-generator-cli-${VERSION}.jar" generate --input-spec api/openapi.yaml --generator-name rust-server --output generated/ --package-name reposync-lib
