def buildImage
pipeline {
    agent {
        node {
            label 'rust-agent-latest'
        }
    }
    environment {
        BASE_URL="registry.dev.zextras.com/infra/"
        IMAGE_NAME="reposync"
        DOCKER_ARGS="--no-cache . -f deployment/Dockerfile"
        IMAGE_VERSION="${TAG_NAME}"
    }
    options {
        buildDiscarder(logRotator(numToKeepStr: '20'))
    }
    stages {
        stage('Build') {
            steps {
                sh 'cargo build --release'
            }
        }
        stage('Tests') {
            steps {
                sh 'cargo test'
            }
        }
        stage('Docker') {
            when {
                buildingTag()
            }
            stages {
                stage('Build Release Image') {
                    steps {
                        dir('deployment') {
                            script {
                                buildImage = docker.build("${BASE_URL}${IMAGE_NAME}:${IMAGE_VERSION}", env.DOCKER_ARGS)
                            }
                        }
                    }
                }
                stage('Push Image') {
                    steps {
                        dir('deployment') {
                            script {
                                buildImage.push(env.IMAGE_VERSION)
                            }
                        }
                    }
                }
            }
        }
    }
    post {
        always {
            script {
                GIT_COMMIT_EMAIL = sh (
                        script: 'git --no-pager show -s --format=\'%ae\'',
                        returnStdout: true
                ).trim()
            }
            emailext attachLog: true, body: '$DEFAULT_CONTENT', recipientProviders: [requestor()], subject: '$DEFAULT_SUBJECT',  to: "${GIT_COMMIT_EMAIL}"
        }
    }
}
